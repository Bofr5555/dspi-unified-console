# Platform support

*The `uconsole` terminal client (`apps/terminal/`, package `dspi-term`). Updated
2026-11-07 for this repository at `2f1e95f3`. The earlier version of this page
described DSPi-Terminal before the merge; its runs are kept below, marked as
such.*

## Status

What this repository's checks do today, per target. The steps are in
`ci/check.sh` and run in containers (`ci/run.sh`).

| Platform | Target triple | Built by | Tests run | Hardware pass, this tree |
|---|---|---|---|---|
| Linux x86_64 | `x86_64-unknown-linux-gnu` | `--step terminal` (release binary, runs `--version`), `--step test` | yes, `--step test` | open (TM13, on the K340) |
| Windows (MinGW) | `x86_64-pc-windows-gnu` | `--step windows-gnu` (the whole workspace) | no | no |
| Windows (MSVC) | `x86_64-pc-windows-msvc` | `--step windows-msvc` (cargo-xwin, with `dspi-core` and `dspi-capi`) | no | no |
| macOS | `aarch64-apple-darwin`, `x86_64-apple-darwin` | no step | no | no |
| Linux, Raspberry Pi 3/4/5 | `aarch64-unknown-linux-gnu` | no step | no | no |
| Linux, older Pi | `armv7-unknown-linux-gnueabihf` | no step | no | no |

The terminal's hardware pass on this tree is work package TM13 in
`docs/core-merge/PLAN-CLIENTS.md`, still open. Until it is done, treat every row
as "expected to work, unverified", and run `uconsole doctor` on the machine for the
real answer.

### Earlier runs, before the merge

These ran on DSPi-Terminal or on this repository's old core, not on the merged
tree:

- **macOS on Apple silicon**, 2026-07-27, against `release/v1.1.5`: linked, tests
  run, and run against a device.
- **macOS x86_64**, 2026-11-06, in a macOS VM with a DSPi passed through, read
  commands only: `uconsole list`, `uconsole doctor`, `uconsole ext list`, `uconsole ext vreg`,
  `uconsole ext xfeed-map`, and the interface on `--fixture` in a pty. A universal
  binary was made with `lipo`. Details: `docs/PORTABILITY.md`, section 9.
- **Windows**, 2026-11-06, with the headers of firmware `bofr-main` `af001ec`:
  both builds cross-linked on Linux and run under Wine 11 with no device: `--help`, `--version`, `list`, `doctor`, and the interface on
  `--fixture` in a pty. The MinGW build's test suite passed under Wine. None of
  that touches USB, so WinUSB itself is unverified on real Windows.
- **Raspberry Pi targets** type-checked (`cargo check`) only.

## What each platform needs

### Windows

Nothing to install. The firmware ships MS OS 2.0 descriptors, so Windows binds
WinUSB to the vendor interface automatically; there is no Zadig step.

Two constraints shaped the code:

- **WinUSB caps a control transfer at 4 KB**, and the bulk parameter packet is
  6136 bytes, so the chunked opcodes are used **on every platform**, not just
  Windows. One code path, exercised everywhere, rather than a Windows-only path
  that nobody tests.
- **`nusb` cannot do device-level control transfers on Windows**, and WinUSB
  additionally requires the low byte of `wIndex` to match the claimed interface.
  All transfers therefore go through a claimed interface, and `wIndex` is not a
  parameter anywhere in the transport API. The protocol already requires
  `wIndex == 2`, so these agree.

Terminal support differs sharply between the two consoles, and the code detects
which one it is in: Windows Terminal and ConEmu get truecolor and braille, while
the classic console gets 16 colours and block glyphs, because it renders braille
as boxes in most fonts. Windows sets neither `TERM` nor `COLORTERM`, so the usual
Unix sniffing would have reported 16 colours for Windows Terminal.

### macOS

Nothing to install. Interfaces 0 and 1 belong to the audio class driver;
interface 2 is free.

**The vendor interface is exclusive.** DSPi Console and this tool cannot hold the
same device at once, and the failure surfaces as a permission-style error that
has nothing to do with permissions. `uconsole doctor` says so explicitly rather than
repeating the Linux advice.

### Linux and Raspberry Pi

Needs a udev rule for non-root access:

```sh
sudo uconsole --install-udev
sudo udevadm control --reload
```

The rule it writes is the one rule every component of the project shares (the Qt packages install the same
file; [`../INSTALL.md`](../INSTALL.md#usb-access-on-linux)).

then unplug and replug the device. The rule also covers the RP2 bootloader the
device re-enumerates as during a firmware update; without that, an update appears
to hang at exactly the wrong moment.

`uconsole doctor` checks for the rule and prints the exact fix when it is missing.

### Raspberry Pi specifically

`--lite` reduces the redraw rate: meters at 10 Hz instead of 20, and no
animation. It is chosen automatically on a single-core or ARMv6 part, and over
SSH, where the cost of a repaint is the link rather than the processor.

## Deliberately not assumed

- No C library for USB. USB goes through `nusb`, which talks WinUSB, IOKit and
  usbfs directly. The default `midi` feature is the exception on Linux: `midir`
  needs the ALSA development files there (`--no-default-features` drops it).
- No hardcoded paths outside the Linux-only udev installer.
- Few outside programs, each named here: `uconsole hub` keeps the machine awake
  through `systemd-inhibit` on Linux and `caffeinate` on macOS, and a filter-file
  export stamps its header with `date`.
- `NO_COLOR` is honoured, and `TERM=dumb` degrades to plain ASCII, so output
  captured to a file or a CI log stays readable.

## Verifying a platform

From the repository root:

```sh
ci/check.sh --step terminal                 # Linux release build, in the container
ci/check.sh --crate dspi-term               # format, clippy and tests
cargo check -p dspi-term --target <triple>  # compiles, for a target with no step
uconsole doctor                                 # the real answer, on the machine
```

`uconsole doctor` is the one that counts: it enumerates, opens, claims the interface,
probes the firmware, and reports the terminal's capabilities, with a fix for each
failure written for the platform it is running on.
