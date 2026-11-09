# The Swift console on the K340: hardware record

The Swift app's hardware runs: macOS in the `macr` VM and iOS in the Simulator. SW15, SW16 and SW-M5
(`docs/core-merge/PLAN-CLIENTS.md`) own this file; their own runs (a real iPhone or iPad, an Apple silicon Mac) have not
happened. Facts are from the run reports named below; nothing here was re-run for this record.

## Runs

| run | date | commit | report |
|---|---|---|---|
| hw-mac | 2026-11-08 (00:47–02:15) | `main` @ 2df60015 | `~/.cache/dspi-build/work/hw-mac/REPORT.md` |
| hw-mac-2 (confirms SWF1–SWF3) | 2026-11-08 (03:18–03:50) | `main` @ bc1f9ff4 | `~/.cache/dspi-build/work/hw-mac-2/REPORT.md` |

- **Device:** PCB unit BB66A44F2AC1E7B6, RP2350, firmware 1.1.6 (bofr-main; wire V32), core 1.20 V.
- **Who:** both runs by an agent; no owner hands.
- **VM:** `macr` (macOS 26.7.2, Xcode 26.6), GUI session, patched QEMU and libusb so that 6136-byte control reads pass
  through. The DSPi was attached by QMP `device_add` (usb-host 2e8b:feaa, id `dspi`) and removed with `device_del` at the end.
  One Metal job at a time under the guest lock.
- **iOS:** the Simulator's CoreMIDI does not see the Mac's USB MIDI (it lists only *Network* and *UMP Network*), so the iOS app
  ran over the development relay: `macrelay` on the Mac holds the DSPi's MIDI endpoints and the app is launched with
  `-MIDIRelay 127.0.0.1:5650`. Everything above the port is the code a real iPhone runs; the timings include one TCP hop.
- **Safety:** nothing flashed, bootloader never entered, no preset save, load or delete, signal generator never started
  (Siggen tests excluded, its page not opened). VREG read only on the K340; OVERVOLT went only to a **demo** `dspi hub` in a
  container with no USB device. Only audio: 6 s of digital silence through `flash-tools/guarded-silence.sh`, sink 40 %.
- **Backup and diff:** hw-1's `backup-inner.sh` and `compare.sh` on the host (timestamps and uptime ignored).

## hw-mac

### macOS app, Swift transport

| check | result |
|---|---|
| connect, read state ("DSPi (2AC1E7B6)", Preset 1, −23.0 dB, 3500 Hz HC/LC on I2S 1) | pass |
| overview, channel page (I2S 1 L), graph | pass |
| write: output delay 0 → 1 → 0 ms, read back over MIDI | pass |
| notification of an external MIDI write | pass |
| presets list (read), About ("Version 1.1.6 (1)"), Stats (RP2350, 307.2 MHz, 1.20 V, 48 kHz) | pass |
| `device_del`: "No Devices", views emptied, no crash; `device_add`: full state within 3 s | pass |

### macOS app, `-CoreTransport YES` (SW4: the core over HubTransport over USB; proven by sampling the process)

| check | result |
|---|---|
| connect, overview, channel page, graph | pass |
| write: delay 0 → 1 → 0 ms, read back over MIDI | pass |
| notification of an external MIDI write | pass |
| presets list, About, Stats | pass |
| `device_del` / `device_add`: "No Devices", then reconnected within 4 s | pass |

### Opt-in hardware XCTests (explicit `-only-testing` lists)

| pass | result |
|---|---|
| USB safe set (`TEST_RUNNER_DSPI_USB_HW=1`): HardwareIntegration, Adat, I2sSlave, SpdifInput, Upmixer, the read and restore methods of I2sClockPinMode, Psybass, Subharm, Limiter, Tube, Rta | 72/72 pass; image byte-identical after (USB and MIDI) |
| `PresetDocumentHardwareTests` (3) | "3/3 passed", **but the device was left changed (F1)** |
| `MIDITransportHardwareTests`, read only | pass |
| `ExtensionChannelHardwareTests` (`DSPI_EXT_HW`) | pass: EXT_LIST 1,2,3; pair map changed and restored; VREG read only |
| `MIDITransportWriteHardwareTests` (with a backup dir) | pass: 8 write kinds restored and verified; 0 sequence gaps; ends byte-identical |
| excluded by the safety rules | Siggen, the SaveCoordination flash test, TubeLayoutBenchmark, the ungated setter tests (O1), the RTA config, invalid pin-mode and DisplayConfig tests |

MIDI timing on macOS (VM passthrough): GET p50 0.85–1.1 ms, p95 2.2–2.7 ms; 0xA0 6136 B 24–27.5 ms; SET+GET 1.6–3.3 ms;
notifications 0.8–2.5 ms after the write. Memory's bare-metal figures are 0.3–0.5 ms and 21 ms.

### iOS app (Simulator, via the relay)

| check | result |
|---|---|
| discovery, probe, read state, 0xA0 6136 B over SysEx | pass |
| `MIDITransportHardwareTests` (read only) | pass: 2590 requests, 0 timeouts, 0 malformed, 0 gaps |
| `ExtensionChannelHardwareTests`; `MIDITransportWriteHardwareTests` | pass; ends byte-identical |
| notification of a USB-side write (`out.gain 0` −2 → 0 dB) in the UI | pass, within 4 s |
| timing | GET p50 1.35–2.4 ms, p95 4.4–5.6 ms; 0xA0 38.8–49.8 ms (mean 41.8) |
| screens on iPhone 17 Pro and iPad Pro 11-inch (slide-over 320 pt, split 507 pt), each with a live read: main, input 1, output 1, channel list, graph edit, Devices, Settings, Matrix, Loudness, Crossfeed, Psychoacoustic Bass, Subharmonic, Tube, Upmixer, Leveller, Spectrum, Stats, Interrupts, AutoEQ, Graph, What's New, iPhone landscape | reached, values match the device; **layout defects F5–F8** |
| write from the UI: out1 gain 0 → −1 → 0 dB (Simulator.app with real keyboard input; XCUITest could not commit the field, O8) | pass; image identical after |
| disconnect on `device_del` | pass, indicator only (crossed cable, red dot); no text banner (SW7) |
| reconnect | pass after a relay restart (the relay cannot follow a replug; a dev tool) |
| iOS over Link via the Mac hub | skipped: the hub harness skips on the USB leg (O7) |

### Hubs and clients

| check | result |
|---|---|
| Mac app as hub (Share Devices on This Network), Linux `real_hub` client: pair with PIN, smoke, 7_attach_gets | pass, settings compared; paired as Viewer |
| Mac hub, Linux `dspi`: pair, list, get; write `out.delay 0` 1 → 0 as Control | pass |
| echo of other sessions' and the Mac UI's writes to a `watch` | pass |
| read-only role: set refused ("opcode 0x78 is not allowed"), value unchanged | pass |
| hub restart, token reused; full app restart, token reused; revoke forces pairing again | pass |
| network voltage-raise refusal, Mac hub | not tested on hardware; **F4** from the code |
| `dspi hub` on Linux, Mac app as client: pair with PIN, full state, write 1 ms, echo both ways, hub restart and token reconnect | pass |
| same, Mac client set to viewer, then a UI write | device unchanged: pass; **the UI kept the value (F3)** |
| voltage-raise refusal, Rust hub on `dspi --demo hub --open`, admin client with OVERVOLT: `ext vreg set 1250` and `trial 1250` | pass: refused, still 1.20 V |

### Device restored

| copy | result |
|---|---|
| before | identical to hw-win's final export |
| after `PresetDocumentHardwareTests` | 66 bytes differ (F1). **Restored** with `dspi import before.dspipreset` (RAM only); identical again |
| after the VM | `i2sInputRateHz` 48000 → 96000 (macOS's USB audio driver). **Restored** by 6 s of guarded 48 kHz silence |
| final | **empty diff** against before (`backup/final-diff.txt`, rc 0); uptime without a break, `hang_count 0` |

### Failures

| id | what | owner, fix |
|---|---|---|
| F1 | `PresetDocumentHardwareTests` captured the view model's defaults before the first read and "restored" them onto the device | SWF1 |
| F2 | the macOS app cannot connect over USB while another CoreMIDI client holds the DSPi's MIDI port | SWF2 |
| F3 | a write the hub refuses leaves the Mac UI showing the unapplied value and a dirty preset | SWF2 |
| F4 | the Swift hub had no network voltage-raise rule (an admin client could raise VREG) | SWF2 |
| F5 | iOS Upmixer title overlaps the header | SWF3 |
| F6 | iOS Graph tool collapses to a thin strip | SWF3 |
| F7 | iPhone right-edge clipping (output strip delay and mute, Subharmonic, Tube, Stats, What's New; iPad Psybass) | SWF3 |
| F8 | false "newer firmware" banner on iOS (target version "1.1.6 beta 5") | SWF3 |
| O1 | ungated unit tests write to a real device under `TEST_RUNNER_DSPI_USB_HW=1` | SWF1 |

Observations O2–O12 (the delay field re-sending on blur, PEQ SET with no MIDI notification, SETs at connect in read-only, two
MIDI hosts colliding, RTA started by opening the analyser, `LinkHubHarnessTests` skipping on USB, XCUITest and custom
controls, the hub's attach delay, pairing prompt wording, overview after reconnect, and others) are in the report; hw-mac-2
did not re-check them.

## hw-mac-2: SWF1–SWF3 on the K340

| check | result |
|---|---|
| SWF1: USB safe set plus the two RTA live config tests | 73 pass, 1 skipped (N1); 0xA0 same before and after (USB and MIDI) |
| SWF1: `PresetDocumentHardwareTests` | 3 pass; 0xA0 same (F1 fixed) |
| SWF1: the whole Limiter, Tube, Subharm, Psybass WireTests, the benchmarks, RtaRenderingActiveReopenTests, DeviceImageGuardTests | 79 pass, 1 expected failure (the guard's own test, on the demo device), 7 skipped; 0xA0 same (O1 fixed) |
| SWF1: the per-test image guard | ran on every live test; did not fire. Not tripped on purpose on the K340 |
| SWF2 F2: the app connects over USB while `macrelay` holds the MIDI port; notifications and a write over the shared handle | pass (`ioreg`: MIDIServer owns the device, DSPi Console the vendor interface) |
| SWF2 F3: viewer on a Linux `dspi hub`, delay edit 0 → 1 ms | pass: back to 0.0 ms in ~1 s with "This console's role on the hub does not allow that"; preset not dirty |
| SWF2 F4: Mac hub voltage-raise rule | pass by code and unit tests (28 pass, 1 skipped); the Mac hub can share only a real USB device, so no raise was sent |
| SWF3 F5–F8 on iPhone 17 Pro and iPad Pro 11-inch over the relay | pass; values match the device; 0xA0 same |

Deviation: the first iPad run was void (the relay stopped serving after six iPhone sessions); a relay restart fixed it.
A stale keychain token from hw-mac's build made macOS ask for the login keychain password; it was denied each time (never
typed), the test token deleted and the app paired again (O13). The Local Network prompt was allowed.

### Device restored

`before` identical to hw-mac's final; after the VM `i2sInputRateHz` 48000 → 96000, **restored** by 6 s of guarded silence;
final **empty diff** (`backup/final-diff.txt`, rc 0). Uptime without a break, `hang_count 0`. The DSPi back on the host; the
app's defaults restored; no test keychain item left.

### New findings

| id | what | owner |
|---|---|---|
| N1 | `SpdifInputWireTests/testLiveFetchPopulatesViewModel` always skips on hardware (uses the offline view model) | SWF1; folded into SW8 |
| O13 | a token stored by another ad-hoc build makes macOS ask for the login keychain password | SW6 |
| O14 | `dspi hub` ignores `pair` on a piped stdin | the terminal hub |
| O15 | the dirty baseline adopts external writes, so restoring the stored value reads as a change | SW13 |

## Still owed

- **SW16:** a real iPhone or iPad (USB-C MIDI, Link, background and return); the MIDI timings on a physical device. No device
  has run the app.
- **SW-M5:** an Apple silicon Mac (the VM is Intel).
- iOS over Link via the Mac hub on the K340 (skipped in both runs; O7 not fixed).
- A text disconnect banner on iOS (SW7).
- Still excluded by rule: Siggen, the SaveCoordination flash test, the invalid pin-mode test, `DisplayConfigDiagTests`,
  `LinkFixtureGeneratorTests`. The RTA config restore is outside the 0xA0 image and was not checked independently.
