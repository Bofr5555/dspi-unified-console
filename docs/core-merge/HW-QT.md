# The Qt console on the K340: hardware record (QD24, QD25)

QD24 (Linux) and QD25 (Windows) in `PLAN-CLIENTS.md`. Done when: connect, read, write, notifications, replug, firmware
path and Link pass on the K340 under section 6's rules. QD35's hand checks will get their own section here.
Facts are from the run reports named below; nothing here was re-run for this record.

Device in both runs: PCB unit BB66A44F2AC1E7B6, RP2350, firmware `v1.1.6-20-g04122d3` (bofr-main @ 04122d3, 2026-11-06),
wire V32, 17 channels, core 1.20 V, USB 2e8b:feaa. Both runs were done by an agent; no owner hands.

Safety in both: nothing flashed, bootloader never entered, VREG read only, no OVERVOLT, no preset save, load or delete,
signal generator never started. Only audio: digital silence through `flash-tools/guarded-silence.sh`, sink at 40 %. Backup
and diff with `backup-inner.sh` and `compare.sh` (whole-device export, filters, `cs export`, raw 0xA0 image, names, preset
directory, VREG and crossfeed map read; timestamps and `uptime_s` ignored). See `HW-TERMINAL.md` for the same runs' Terminal
part.

## QD24: Linux (hw-1)

- **Date:** 2026-11-07/08. **Commit:** `main` @ fd3d6219. **Report:** `~/.cache/dspi-build/work/hw-1/REPORT.md`.
- **Setup:** Linux host; the Qt 6 and Qt 5 builds in podman (`localhost/dspi-coremerge:4`). The smoke driver ran with
  `DSPI_SMOKE_HW=1` in a container with only the device node, real sysfs, `--network=none`, no keep-id: `run-hw.sh`'s
  command, reproduced by hand (its `ci/run.sh` build would have used a shared target dir). Logs:
  `smoke-hw-qt6/log.txt`, `smoke-hw-qt5/log.txt`.

### Checks (the same on Qt 6 and Qt 5: 37 checks, 3 failures, 5 not run, 0 QML errors)

| check | result |
|---|---|
| preflight, bulk baseline | pass |
| status and unchanged config, stats, starvation 0, buffer watermarks, system | pass |
| S/PDIF, I2S slave, ADAT in and out, upmixer, LG Sound Sync status, input rate 48 kHz, output latency | pass |
| extension channel (1, 2, 3), meters, control-surface model | pass |
| RTA caps; RTA close stops traffic | pass |
| signal generator caps (idle), still idle at the end | pass |
| write: bypass toggled and restored (off) | pass |
| disconnect, reselect, re-read, re-poll, no connection error | pass |
| "final image identical to the first" (also after the bypass toggle and the reselect) | pass |
| RTA frames, input tap; RTA frames, output tap; RTA configuration (frames after: 1) | **fail ×3, expected**: only silence was played, by policy ("digital silence changes no picture"). Needs a real signal ≤ 80 % through the guarded player |
| IR learn, watermark reset, clock diagnostics, siggen start, bypass repaint | not run, by the driver's own rules (no IR receiver; irreversible; not built in; makes sound; no live PEQ band on IN1/IN2) |
| notifications from another client | **not checked**: the smoke has no external-write test |
| replug | **not run**: needs the owner's hands (`run-replug.sh`) |
| firmware path | **not run**: flashing was forbidden in this job |
| Link from the Qt client | **not run**: Link was covered with the Terminal only (`HW-TERMINAL.md`) |

### Device restored

Final export against the first backup: **empty diff** (`backup/final-diff.txt`, rc 0); no restore needed. Uptime without a
break, `hang_count 0`.

## QD25: Windows (hw-win)

- **Date:** 2026-11-08. **Commit:** `main` @ fd3d6219. **Report:** `~/.cache/dspi-build/work/hw-win/REPORT.md`.
- **Setup:** VM `dspi-win11` (`winvm`, Windows 11 Pro 26300, WARP), the DSPi passed with a stock `usb-host` hostdev
  (`dspi-usb.xml`), detached at the end; the bootloader device was not attached (no flash). App: DSPiConsole 1.1.6 release
  zip from `apps/desktop/packaging/windows/build-windows.sh` (MinGW Qt 6, in a container). The smoke driver does not build for
  Windows (F3), so the app was driven by QMP clicks, `send-key` and `virsh screenshot`, in the interactive session through a
  scheduled task. While the app held the USB interface, the device was read with `dspi.exe --midi` (WinUSB is exclusive).

### Checks

| check | result | evidence |
|---|---|---|
| device found | pass: "DSPi (2AC1E7B6)"; `dspi.exe doctor` meanwhile gets "failed to open device (error 5)", as an exclusive open should | `qt-05-real.png` |
| overview | pass: Preset 1, Source USB, −23.0 dB, IN1/IN2 USB, OUT1–4 I2S, the 3.5 kHz crossover | `qt-06-overview.png` |
| channel pages, graph | pass: OUT1 (graph, gain and delay, PEQ with HC 3500 Hz Q 0.707), IN2 | `qt-07-channel-out1.png`, `qt-15b-overview.png` |
| write: OUT1 delay 0 → 1 → 0 ms | pass: GUI 1.00 ms and preset marked modified; device read 1.0 ms over MIDI; back to 0.0 and the mark clears | `qt-08`…`qt-10` |
| notifications: the app follows another client's write | **fail (F1)**: delay set to 1 and 2.5 ms over MIDI; the notification arrived and the preset was marked modified, but the field kept 0.00 ms | `qt-11`…`qt-15`, `qt-21-midi-2p5.png` |
| presets list (read) | pass: 1 "Preset 1", 2–10 "Empty" | `qt-22-presets.png` |
| About | pass: "Version 1.1.6 for Windows" | `qt-26-settings.png` |
| System Statistics | renders; S/PDIF DMA starvation shows huge totals (O2) | `qt-23-about.png` |
| detach (`virsh detach-device`) | pass: within 4 s the "No DSPi connected… [Open Demo]" banner, empty sidebar, "No Devices" | `qt-27-detached-4s.png` |
| re-attach | pass: everything back within 3 s | `qt-29-reattach-grid.png` |
| demo mode | pass | `qt-04-demo.png` |
| launch time | observation O1: usually 2–5 s, but 19–49 s or apparently stuck on some first launches | `relaunch.log`, `timelaunch.log` |
| smoke driver (`DSPI_SMOKE_HW=1`) | **fail (F3)**: does not build for MinGW | `smoke-win-build.log` |
| firmware path | not run: flashing forbidden in this job |  |
| Link from the Qt client, the Qt app's own hub | not run. `dspi.exe hub` from the VM passed with Linux Terminal clients (`HW-TERMINAL.md`) |  |

### Device restored

First backup byte-identical to hw-1's final export. After the VM, one difference: `i2sInputRateHz` 48000 → 96000 (Windows'
USB audio driver; `dspi set in.rate 48000` refused). **Restored** by 6 s of guarded 48 kHz digital silence on the host;
final **empty diff** (`backup/final-diff.txt`, rc 0). On re-attach Windows also rewrote the user volume (−60 → −22 → −23 dB),
ending at the original. Uptime without a break (14422 s → 16318 s), `hang_count 0`. VM shut down cleanly, the DSPi back on
the host.

### Failures and where they went

| id | what | fixed |
|---|---|---|
| F1 | the Qt app did not show a channel delay written by another client (the image holds an output's delay twice; a notified write patched one copy) | CG31 (25990e0d); not re-run on the K340 |
| F3 | `DSPiConsoleSmoke` is Linux-only (DBus, `pipe2`, the D-Bus secret-service fake) | open, QD25 |
| O1 | slow first starts in the VM, cause not found | open; time it on real Windows hardware |
| O2 | S/PDIF DMA starvation counters for instances 3 and 4 read huge in Windows, 0 on Linux | a firmware question |
| O3 | the Windows USB audio driver sets `in.rate` to 96 kHz and rewrites the user volume | expected; a future run needs the same restore |

### Deviations

- No smoke driver (F3): the app was driven by hand.
- Windows' sound events were silenced for the run (the DSPi became the default output) and restored.
- The app ran with the tester profile's existing `DSPi Console.ini`; UI settings it changed were left.

## Still owed

- **QD24:** replug (`run-replug.sh`, owner's hands); Link from the Qt client on the K340; notifications from another client
  (no smoke check exists on hardware; CG31 added one on the fake hub); the firmware path (flash: coordinator and owner's go,
  backups first); the three RTA items with a real signal ≤ 80 %.
- **QD25:** a re-run of the notification check after CG31; Link from the Qt client and the Qt app's hub; the firmware path
  (with the bootloader device attached, as QD25's setup asks); F3 (smoke on Windows); O1 on real hardware.
