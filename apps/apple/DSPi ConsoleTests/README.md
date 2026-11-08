# DSPi ConsoleTests

Headless verification for the **app** (not the firmware). Drives the real Swift
command layer so current and future features can be checked without clicking the
GUI. The firmware control plane has its own suite at `DSPi/tools/dspi_test`; this
complements it by exercising what *that* suite can't reach - the app's own USB
encoding, response parsing, response math, and state logic.

## Two layers

- **Pure-logic** (`DSPMathTests`, `PresetSnapshotTests`) - no device needed.
  `DSPMath` magnitude/phase against analytic landmarks (peak gain at fc, -3 dB at
  a Butterworth cutoff, -6 dB at an LR crossover, all-pass -90 deg), and
  `PresetSnapshot.diff` unsaved-change detection. Runs anywhere, including CI.
- **Live-device** (`HardwareIntegrationTests`) - drives the real `DSPViewModel`
  setters (the same code the UI buttons call), then reads the value back over USB
  with an **independent** raw control transfer (`HardwareTest.readFloat/readU32`)
  so a self-consistent set/get bug can't make a round-trip falsely pass. Each test
  snapshots the original value and restores it. These **skip** (never fail) when
  no DSPi is attached.
- **Save coordination** (`SaveCoordinationTests`) - the logic between the Settings
  UI and the device: preset unsaved-change detection (`DSPViewModel`), and the
  global / output-config save batching and mode gating (`SettingsSaveCoordinator`).
  Most of it needs no device or flash (dirty detection, the independent-vs-with-
  preset save gating). One opt-in test exercises the real `save()` flash write -
  see below. It does **not** cover the literal `NSAlert` modal or that controls are
  wired to these methods; that last mile would need an XCUITest target.

## Run

```bash
# Whole suite (Xcode 26.2 is required to link DerivedData built by it):
DEVELOPER_DIR=/Applications/Xcode-26.2.0.app/Contents/Developer \
  xcodebuild test -project "DSPi Console.xcodeproj" -scheme "DSPi Console" \
  -destination 'platform=macOS'

# Pure-logic only (skip the hardware group - e.g. on CI with no device):
#   ... the same command; HardwareIntegrationTests self-skip when no device.

# One test or class:
#   ... add: -only-testing:"DSPi ConsoleTests/DSPMathTests"

# Include the opt-in flash-writing save test (writes flash; needs a device).
# The flag is a GLOBAL default because the host-app test process inherits neither
# the shell env nor the app's own defaults domain, but does read NSGlobalDomain:
defaults write -g DSPiAllowFlash -bool YES
DEVELOPER_DIR=/Applications/Xcode-26.2.0.app/Contents/Developer \
  xcodebuild test -project "DSPi Console.xcodeproj" -scheme "DSPi Console" \
  -destination 'platform=macOS'
defaults delete -g DSPiAllowFlash
```

`testGlobalSaveAppliesOutputConfigModeAndClearsDirty` is skipped unless the
`DSPiAllowFlash` global default is set, because it persists to flash (limited
write cycles). It flips the output-config mode through `save()`, confirms the
device persisted it and the dirty flags cleared, then restores the original.
It flips the output-config mode through `save()`, confirms the device persisted
it and the dirty flags cleared, then restores the original.

`xcodebuild test` launches the app as the test host. Under XCTest the app opens
no real device (USB or MIDI) unless the run opts in (`TestHostGuard.swift`, WP SW2),
so live-device tests need `TEST_RUNNER_DSPI_USB_HW=1 xcodebuild test ...`; the MIDI
hardware tests' own `DSPI_MIDI_HW=1` / `DSPI_EXT_HW=1` also opt in. With it,
`AppState.shared.usb` connects to the device during the run. The first live-device test waits up to
30 s for that connection (it can take ~15 s under `xcodebuild test`); the result
is cached so the rest are instant, and a device-less run skips after one probe.

## A hardware run leaves the device as it found it (WP SWF1)

- `requireDevice(self)` waits until the app's view model has finished a full
  read of the device (`fullReadGeneration`), so a capture from it is the
  device's state and not its defaults. It then takes the raw REQ_GET_ALL_PARAMS
  image, and the test **fails** if the image differs after it (after its body
  and `defer`s, before `tearDown`): `DeviceImageGuard`, proven on the demo device
  in `DeviceImageGuardTests`.
- A test of the view model's own logic uses `HardwareTest.offlineViewModel()`,
  never `DSPViewModel()`: the default drives `AppState.shared.usb`, which is the
  real DSPi under the opt-in, and bounces it at init.
- A test that drives the app's shared view model (windows, benchmarks) calls
  `try HardwareTest.requireNoDevice()` and skips in a hardware run.
- `SiggenHardwareDiagTests` play a signal: they need `TEST_RUNNER_DSPI_SIGGEN_HW=1`
  as well.

The guard sees the 0xA0 image only. State outside it (the analyser's
configuration, the generator, control-surface records) is still each test's to
restore.

## Adding a test for a new feature

1. **Pure logic** (math/state): assert against an analytic expectation or a
   reference computed independently of the code under test.
2. **A control that talks to the device**: in `HardwareIntegrationTests`, call
   `try HardwareTest.requireDevice(self)`, save the original via a raw read, call the
   `DSPViewModel` setter, read back with a raw `getControlRequest`, assert, and
   restore the original in a `defer`. The serial queue guarantees a sync read
   right after an async set observes the new value.


## RTA V3 (2026-09-12)

`RtaWireTests` covers the 82-byte/37-slot band frames, capability-derived strides,
10 Hz band indices, continuous bass visibility across all FFT sizes, separate
bass dynamic range and CPU timing, saturation, and rejection of incompatible
protocols. The raw FFT geometry tests remain separate: V3 adds bass RTA bands,
not low-frequency FFT bins. Live tests need matching V3 firmware.

The focused V3 run excludes the four `testLive…` methods. If the existing
`ControlSurfacesWireTests.swift` references removed auxiliary constants, add
`EXCLUDED_SOURCE_FILE_NAMES=ControlSurfacesWireTests.swift` to the focused
xcodebuild command; this bypasses that unrelated compile failure without
changing the Control Surfaces tests.
