# What stays Swift (SW-M4)

An audit of every Swift source file of the Apple app (`apps/apple`) that is not a thin UI over the Rust core. Each
file gets one class: UI, OS integration, opt-out legacy path, or protocol/logic that should be in the core. The
last class comes with file:line evidence and a proposed follow-up WP. This is SW-M4 in
`docs/core-merge/PLAN-CLIENTS.md` section 5.4.

**Baseline.** Audited on `main` at `ab475933` (2026-11-15). SW-M2 (firmware) and SW-M3 (hub) are in flight on
other branches. Their areas are audited as they are on `main` and marked **changing in SW-M2** or **changing in
SW-M3**. SW7 (the banner) is not on `main` either, and adds only UI (`Core/LinkBannerView.swift`).

**Paths.** Swift paths are relative to `apps/apple/DSPi Console/` unless they start with `apps/`. Rust paths are
relative to `crates/`. Line numbers are those on `ab475933`.

## 1. Summary

136 source files: 135 under `DSPi Console/` (with `iOS/`) and `Tools/UpdateSigning/update-signing.swift`. The 127
files in `DSPi ConsoleTests/` and the 10 in `DSPi Console iOSUITests/` are tests and are not counted. `DSPi Console
iOS/` holds only `Info.plist`.

| Class | Files | Lines | Of these, changing in SW-M2 / SW-M3 |
|---|---:|---:|---|
| UI (views, view state, app glue over the core) | 68 | 37 186 | 3 SW-M2, 2 SW-M3 |
| OS integration | 27 | 6 098 | 1 SW-M2, 2 SW-M3 |
| Opt-out legacy path (`-CoreTransport NO`, the Swift hub, the Swift demo) | 27 | 12 122 | 17 SW-M3 |
| Protocol/logic that should be in the core | 14 | 29 590 | 1 SW-M2 |

A file goes in the last class when any of its code runs **on the default core path** and does protocol, wire
decoding, or device or DSP logic that the core has, or should have. The large files there (`DSPi_ConsoleApp.swift`,
`DSPViewModel.swift`, `Commands.swift`) are mostly UI or legacy. Only the ranges in section 4 are the problem.

**What the named items are.** Each was checked, and none carries protocol:
- menu-bar mode and the login item (`MenuBarMode.swift`);
- keep-awake (`Link/LinkService.swift:229-237`, inside the Swift hub: changing in SW-M3, see section 3);
- `AppUpdateChecker` and `AppUpdateInstaller`;
- `HostAudioFormatMonitor`;
- the Metal renderers;
- Bonjour (`HubBrowser`'s `NWBrowser`);
- Keychain (`LinkTokenStore`);
- the CoreMIDI seams;
- the IOKit seam (`BootloaderLocator`, changing in SW-M2).

`HubBrowser` also decodes TXT records and `/info` replies, and parses typed addresses, which is protocol
(item M10).

**The done check fails today.** A grep for opcodes outside `Core/` finds 805 lines in 30 files (section 5). Nearly
all of them are the opt-out path and the Swift hub. Nineteen are opcodes used as keys in five views (M7). One
follow-up WP empties the view part, and the opt-out's retirement and SW-M3 empty the rest.

**Should move to the core: twelve items** (section 4). Five need nothing new in the core, only Swift changes:
M4 (filter files), M7 (opcode keys), M8 (version match), M11 (AutoEQ rebuild), and the first half of M3 (the
image bytes that already have `DspState` fields). The others need a small core export or field first.

## 2. Classes

- **UI.** Views, view models, view state, menus and dialogs. Also the glue that turns core records into view
  state: `Core/CoreMirror.swift` apart from M3, `Core/CoreSnapshots.swift`, `Core/CoreControlSurfaces.swift`, and
  similar files. `CoreBackedTransport` is the seam itself, and it holds no protocol on the device's behalf. Its
  `HostBridge` (`Core/CoreBackedTransport.swift:1335-1459`) carries the bulk image across the Swift hub until
  SW-M3.
- **OS integration.** Code that exists because an Apple API owns the feature: windows and the menu bar, login
  items, App Nap and sleep, CoreAudio, CoreMIDI, IOKit, Network.framework browsing, DNS-SD, the Keychain,
  URLSession for the core's fetch callbacks, Metal, document pickers, update download and verify, and the test
  host.
- **Opt-out legacy.** Code reached only with `-CoreTransport NO` or `DSPI_CORE_TRANSPORT=0`
  (`DSPi_ConsoleApp.swift:218-240`, `AppState.usesCoreTransport`). That covers Swift's own USB, MIDI, Link-client
  and demo engines, and every raw `else` branch behind `typedCore`, `coreStateSource` or `typedWrite`. It also
  covers the Swift hub, which is still on the Mac's default local path. The core drives the device through
  `HubTransport`, then the hub's session 1, then `CoreUsb` (SW-M1). The hub retires in SW-M3.
- **Should be in the core.** Code on the default path that decodes wire bytes, keys behaviour on opcodes or wire
  versions, or duplicates a rule or computation that the core has (or another client has, and the core should
  have).

## 3. OS integration, legacy, SW-M2 and SW-M3

### 3.1 OS integration (27 files): stays Swift

None of these files uses an opcode or a raw transfer. The only one with byte arithmetic is `CoreUsb.swift`: it
moves pipe bytes for the core, as `CoreMidiPipe.swift` does for MIDI.

| File | What it integrates | Notes |
|---|---|---|
| `MenuBarMode.swift` | Menu-bar service mode (no Dock icon). Login item via `SMAppService.mainApp` (`:79-85`) | The menu shows the hub's state. Re-pointed with SW-M3, no protocol |
| `Link/LinkService.swift` | Sharing preferences, the WebSocket server's start and stop, keep-awake (`ProcessInfo.beginActivity`, `:229-237`) | **Changing in SW-M3.** The power assertion and the preferences must survive the Swift hub's removal |
| `Link/LinkDiscovery.swift` | DNS-SD advertisement (`DNSServiceRegister`), TXT encoding | **Changing in SW-M3.** It stays unless `dspi-hub` advertises by itself on macOS |
| `AppUpdateChecker.swift`, `AppUpdateInstaller.swift`, `../Tools/UpdateSigning/update-signing.swift` | GitHub release feed (off: `enabled = false`), Ed25519 check, bundle swap after exit, release signing | App distribution, not device protocol |
| `HostAudioFormatMonitor.swift` | CoreAudio channel count of the DSPi's output stream | Host side only |
| `PeqGraphRenderer.swift`, `PeqGraphShaders.swift` | Metal PEQ graph: GPU evaluation of biquad sections per pixel | The sections come from Swift's `DSPMath` (M5) |
| `RtaMetalSurface.swift`, `RtaMetalBars.swift`, `RtaMetalCurves.swift`, `RtaBarShaders.swift`, `RtaCurveShaders.swift`, `RtaRenderCache.swift` | Metal analyser | They take `RtaSnapshot` (see M12d) |
| `Link/LinkTokenStore.swift` | Keychain tokens per hub id | Used by `Core/CoreLink.swift` |
| `Link/LinkClientCoordinator.swift` | Client preferences, owns `HubBrowser`, manual hub list | Uses `HubBrowser.parseAddress` (M10) |
| `MIDI/MIDIControlPort.swift`, `MIDI/CoreMidiPipe.swift`, `MIDI/MIDITransport.swift`, `MIDI/MIDIRelayPort.swift` | CoreMIDI ports, the core's `MidiSender` and `MidiInputHandle`, endpoint list and hot-plug, the development relay | `ci/check.sh`'s SW5 guard keeps the core's MIDI handle in `CoreMidiPipe.swift` |
| `Core/CoreUsb.swift` | The core's `UsbPipe` (nusb, `macos_iokit`): device list, hot-plug poll, open retries, notification reader thread | No IOKit vendor path left (SW-M1) |
| `BootloaderLocator.swift` | IOKit count of BOOTSEL boards (`:156-161`), mount table (`getmntinfo`, `:184-191`) | **Changing in SW-M2:** it becomes the core's `bootloaderCounts()` and `bootloaderDrives()` |
| `AutoEQ/AutoEQCoreUpdate.swift` | URLSession behind the core's `AutoEqFetch` (CG6) | iOS only today; see M11 |
| `iOS/DocumentPicker.swift` | `UIDocumentPicker` | |
| `AppIdentity.swift`, `TestHostGuard.swift` | Bundle and log names; XCTest host guard | |

### 3.2 Opt-out legacy (27 files): retires with `-CoreTransport NO`

Everything here runs only on the opt-out, or only in the Swift hub (marked SW-M3).
- **Swift engines:**
  - `Link/LinkClient.swift`, `Link/LinkClientProtocol.swift`, `Link/NetworkTransport.swift` (the Link client);
  - `MIDI/LegacyMIDITransport.swift` and the codec half of `MIDI/MIDIControlCodec.swift` (frames, CRC, 7-bit
    packing; the UMP half is I/O and moves to `CoreMidiPipe`'s side when the codec goes);
  - `Demo/FakeTransport.swift` (Swift's demo; the default demo is the core's).
- **Raw branches inside core-aware files:**
  - `ExtensionCommands.swift`;
  - `SpectrumAnalyser.swift` (the raw RTA engine; `:938-985`, `:1319-1340`);
  - `PresetSnapshot.swift` (the Swift unsaved-changes snapshot; the core's `presetChanges` is used when attached,
    `DSPViewModel.swift:3231-3243`);
  - `StatsView.swift`'s raw reads (`:410-480`, `:625-840`), counted as UI.

  Each one checks for a core first, and `funcs.py` in `~/.cache/dspi-build/work/wp-SWM4/` lists every function
  with a raw transfer. Apart from M8's `fetchPlatform`, every unguarded raw function is called only from a raw
  branch: `pollCsDeferred`, `loadPresetRaw`, `waitForPresetActivation`, `saveOutputConfigRaw`, StatsView's
  `fetch*`, and `readBinFrame`.
- **Wire constants:** `Constants.swift`, 797 hand-written `let`s, 238 of them opcodes. 283 of its names are also in
  the generated `DspiConstants` (SW25).
- **The Swift hub, changing in SW-M3** (`Link/`):
  - `LinkHub`, `CommandRouter`, `DeviceRegistry`, `SnapshotCache`, `NotifyRelay`, `PollScheduler`, `LinkPolicy`;
  - `AuthStore`, `LinkSessionHandler`, `LinkServer` (NIO), `LinkFrames`, `LinkMessages`, `LinkRole`;
  - `HubDevice`, `USBHubDevice`, `HubTransport`;
  - `CoreVoltageRule`, a port of `dspi_session::overvolt`.

  `LinkFrames`, `LinkMessages` and `LinkRole` are shared with the legacy client. `NetworkingSettingsView` uses
  `LinkRole` as picker tags.

**What retiring the opt-out takes.** Delete the files above apart from the hub. Delete the raw `else` branches in
`Commands.swift` (192 raw calls), `StatsView.swift` (28), `SpectrumAnalyser.swift` (9) and `ExtensionCommands.swift`
(4). Delete the wire structs in `DSPViewModel.swift:60-1310` (`fromData`/`toData` for about 30 packets), the bulk
decoder (`Commands.swift:4568-5092`) and the notification mirror (`Commands.swift:5606-5929`). Move `Constants.swift`'s
shared names to `DspiConstants`. The comments in `Commands.swift:331`, `:1027` and `:4578` still describe the time
before the core was the default ("USB without `-CoreTransport`"). This is proposed as **SWS13** in section 4.

### 3.3 SW-M2 and SW-M3 on `main`

- **SW-M2.**
  - `FirmwareInstaller.swift` still writes the UF2 itself (`FileHandle` at `:439-463`) and judges the reboot
    (`isRebootSignal`, `:190`): protocol, counted in the last class.
  - `BootloaderLocator.swift` counts BOOTSEL boards over IOKit.
  - `FirmwareInstallUI.swift`, `FirmwareUpdateView.swift` and `Onboarding/GettingStartedView.swift` are its UI.

  `wp/SWM2` (60cc2890) makes the write the core's `flashConfirmed` and the locator the core's. Its own report
  leaves one item, folded into M8: `ViewModelFirmwareVerifier` compares versions in Swift.
- **SW-M3.** The 17 hub files above, `HostBridge` in `CoreBackedTransport.swift`, `NetworkingSettingsView.swift`,
  `LinkService.swift` and `LinkDiscovery.swift`. Two OS duties live inside the Swift hub today and need a home
  after it: keep-awake (`LinkService.swift:229-237`) and the DNS-SD advertisement (`LinkDiscovery.swift`).

## 4. Protocol and logic that should be in the core

"Core today" says what the core already exports, which decides whether the follow-up is Swift-only. The WP ids
`SWS1`-`SWS13` are proposals for the coordinator; sizes are estimates.

### M1. The raw notification decoder still runs on the core path

- **Evidence:** `InterruptMonitor.swift:780-958` (`processEvent`) decodes each raw packet that the transport's
  notification tap delivers (byte 0 = version 2, byte 1 = event id), then calls the handlers that
  `DSPViewModel.swift:2731-2880` installs. Only some handlers return early when a core is attached
  (`onParamChanged`, ADAT, the I2S slave, ADAT input, names, the device name). These handlers act on Swift's
  decode **with a core attached**:
  - `onInputFormatChanged` (`DSPViewModel.swift:2740`, packet bytes at `InterruptMonitor.swift:829`);
  - `onSiggenState` (`:2769`; bytes 4-7 at `InterruptMonitor.swift:841-851`);
  - `onCsAux` (`:2846`; bytes 4-8 at `:900-912`);
  - `onPresetLoaded`, which sets `activePresetSlot` before its core guard (`:2859-2861`);
  - `onBulkInvalidated`, which runs `revertToCore` (`:2872-2878`);
  - `onRtaConfig` (`RtaConfig.fromNotification`, `SpectrumAnalyser.swift:59`).

  The core's monitor log is the only part that reads the core (`:943-948`).
- **Core today:** the event flags exist: `NOTIFY_INPUT_FORMAT`, `NOTIFY_PRESET`, `NOTIFY_SIGGEN`, `NOTIFY_CS_AUX`
  and `NOTIFY_REFRESHED` (`DspiConstants`, `consts.rs`), and the analyser has its listener and ownership read.
  Their values are partly in `DspState` (`activePresetSlot`, `numInputChannels`) and partly behind typed reads
  (`siggenGetStatus`, `csSnapshot`). It is not checked whether every value is there.
- **Proposed: SWS1, "Events from the core" (Swift, 0.5–0.75).** With a core attached, those handlers follow the
  core's flags (`CoreMirror`'s `followCoreEvents`) and read their values from the state or the typed reads. Swift
  decodes no packet unless the device is on the opt-out. If a value is missing, the WP asks for a CG item to add
  it to the event or the state.

### M2. Feature gates and the platform table from wire versions and platform names

- **Evidence:**
  - `DSPViewModel.swift:2151-2262`: thirteen `firmwareSupports*` gates written as `firmwareWireFormatVersion >= N`
    (13, 14, 28, 18, 19, 20, 22, 23, 29, 30, 31, 32, and 26 with RP2350).
  - `Core/CoreMirror.swift:930-947`: the same kind of gates, applied after each image.
  - The platform shape keyed on the platform name: `DSPViewModel.swift:1551`, `:1554`, `:1978`, `:1985`
    (`chOut1`), `:2399-2406` (outputs, PDM index, EQ worker range, output slots), `Commands.swift:2613`, `:2638`,
    `:2726`, `:2739`, `StatsView.swift:568`, `:681`, `PresetSnapshot.swift:388`, `:515`,
    `PresetDocumentTransfer.swift:931`, `DSPi_ConsoleApp.swift:8582`, `:8633`.
  - Platform id to name, written out twice: `Commands.swift:166-171` and `Core/CoreMirror.swift:810-816`.

  Through the core, `firmwareWireFormatVersion` is the core's `formatVersion` (`CoreMirror.swift:915`), so these
  gates run on the default path.
- **Core today:** `wireFeatures()` and `featurePresent(name:)` (`device.rs:466`) give the gates, and
  `DspState.numChannels`, `numOutputChannels` and `numInputChannels` give the shape. Some names may be missing
  (first-order shelves and passes, the leveller and loudness masks).
- **Proposed: SWS2, "Gates and shape from the core" (Swift 0.5, plus a CG of about 0.25 for missing feature names
  and a platform record).** With a core attached, each gate reads `featurePresent` and each count reads the state.
  The version numbers stay only on the opt-out.

### M3. Byte offsets into the core's bulk image

- **Evidence:**
  - `Core/CoreMirror.swift:661-668` (clock-pin mode and slave BCK presence);
  - `:675-698` (S/PDIF input config bytes +4, +8+i, +11);
  - `:711` (ADAT out pin);
  - `:717-720` (ADAT input enabled and clock mode);
  - `:798-807` (`raw()`, `decoded()`);
  - `:944-945`;
  - `PresetDocumentTransfer.swift:20-28` (`customCrossfeedPair`: two little-endian floats at
    `BULK_CROSSFEED_OFFSET + 4` of `bulkRaw`, run for every export through the core).

  The file's own comment (`CoreMirror.swift:661-663`) calls this "a CG candidate".
- **Core today:** `DspState` already has `i2sClockPinModePresent`, `spdifInputsEnabledPresent`,
  `adatInputEnabledPresent` and `adatInputClockModePresent`, so four of these reads are Swift-only fixes. The rest
  have no field: the presence of the slave BCK pin and the S/PDIF pins, the I2S input channels, the ADAT out pin
  default, and the custom crossfeed pair (`crossfeedFreq` and `crossfeedFeed` give the named preset's values).
- **Proposed: SWS3, "No image bytes in Swift" (Swift 0.25, plus a CG of about 0.25 for the missing fields).**
  Afterwards no Swift file reads `bulkRaw` except `decoded()`, and that could become a `DspState` flag.

### M4. Filter-file import and export

- **Evidence:**
  - `FilterFile.swift:1-271` (the text grammar, parse and format);
  - `DSPi_ConsoleApp.swift:10670-10719` (Mac import and export);
  - `:11005-11201` (`parseREWFile`, `parseDSPiFile`, `generateExportString`);
  - `:11258-11435` (channel choices, apply, unsupported types);
  - `:11452-11505` (iOS).

  These run with a core attached: neither import nor export checks for one.
- **Core today:** `filtersExport(inputs:)`, `filtersInspect(text:inputs:)` and `filtersImport(text:wires:)`
  (`dspi-core/src/files.rs:3344-3376`). The import holds the device lock and applies through the core's setters.
  Qt uses them.
- **Proposed: SWS4, "Filter files through the core" (Swift, 0.5).** The pickers keep their panels. Parse, apply and
  export become the core's three calls. `FilterFile.swift` stays only for the opt-out, or goes.

### M5. Response curves computed in Swift

- **Evidence:**
  - `DSPMath.swift:485-896`: `responseAt`, `phaseAt`, `unwrapPhase`, `calculateCoefficients` (RBJ, first-order,
    Linkwitz), `crossoverSections`, and `sampleRate`.
  - The callers on the core path: `DSPViewModel.swift:3074-3093` (`recomputeMagnitudes`, 201 log-spaced points
    from 10 Hz to 20 kHz), `PeqGraphModel.swift:146-153` and `:323-344`, `PeqGraphEditor.swift:420-427`,
    `iOS/PeqTouchEditor.swift:102`, and `PeqGraphRenderer.swift:395-408`, which feeds `DSPMath.Coeffs` to the
    Metal shader.
  - `DSPMath.swift:3-327` defines `FilterType` with the firmware's raw values.
- **Core today:** `computeMagnitudeCurve`, `computePhaseCurve(unwrap:)` and `computeResponse(freq:)` (`curves.rs`,
  the same 201 points as `dspi_proto::dsp::POINTS`), plus `curveFreq` and `filterTypeNames`. Android's graph uses
  them (`apps/android/bridge/.../DspiBridgeImpl.kt:459`). The core does not export the biquad sections that the
  GPU path needs.
- **Proposed: SWS5, "Curves from the core" (Swift 0.75, plus a CG of about 0.25 that exports a band's sections,
  `curve_sections(filters) -> [Coeffs]`).**
  - The CPU curves and the hit-test probes call the core.
  - The Metal renderer keeps its phi-form conversion and per-pixel evaluation (that part is the renderer), but
    takes the core's coefficients.
  - Run a benchmark first: the editor probes `responseAt` on every drag event.

### M6. Rounding rules copied from the core

- **Evidence:**
  - `Commands.swift:393-401` (`keptMasterVolume`: "the core's own rule (0.5 dB steps, -128 mutes)", written out)
    and `:403-416` (the Swift steps for the opt-out);
  - `Commands.swift:639-642` (`quantizeDelayMS`, the core's 0.0001 ms grid, written out; used at `:436`, `:645`,
    `:2244`);
  - the 0.1 dB gain rounding repeated in the setters (`Commands.swift:351`, `:362`, `:423`, `:657`, `:673`, `:2212`).
- **Core today:** `quantizeDelay(ms:)` and `quantizeGain(db:)` are exported (`curves.rs:388`). The master-volume
  rule is applied inside `setMasterVolume` and not exported.
- **Proposed: SWS6, "The core's quantizers" (Swift 0.25, plus a CG one-liner that exports
  `quantize_master_volume`).**

### M7. Opcodes as keys for live slider sends

- **Evidence:** `Commands.swift:1180-1219`. `sendFloatParamToDevice(_ request: UInt8, ...)` takes an opcode, and
  `typedFloatSet` maps 18 opcodes to a core setter and lane key. The views pass the opcodes:
  - `CrossfeedView.swift:297`, `:320`;
  - `LoudnessView.swift:255`, `:270`;
  - `VolumeLevellerView.swift:144`, `:183`, `:198`;
  - `PsychoacousticBassView.swift:334`, `:349`, `:372`, `:388`, `:402`;
  - `SubharmonicSynthView.swift:373`, `:385`, `:398`, `:482`, `:494`, `:545`, `:571`.

  These are 19 of the opcode lines outside `Core/` (section 5), on the default path.
- **Core today:** nothing is missing. The lane keys are the core's registry paths (`"loud.ref"`, `"sub.low"`, ...).
- **Proposed: SWS7, "Live sends by setting, not opcode" (Swift, 0.25).** Use a Swift enum of the live settings
  (or the registry path). The opcode appears only in the opt-out's raw branch. This empties the view part of the
  grep.

### M8. Firmware identity and version match

- **Evidence:**
  - `Commands.swift:157-180`: `fetchPlatform` sends a raw `REQ_GET_PLATFORM` on every refresh (`:35`), with or
    without a core, through the core's `rawGet`. `waitForDeviceReady` (`:127-136`) uses it as its readiness
    probe. SW-M1's report lists both.
  - `FirmwareVersion.swift:98-113` (`fromPlatformReply`, the reply layout and the early-beta rule).
  - `DSPViewModel.swift:2116-2132` (`firmwareMatch`: compares with the app's expected version in Swift).
  - SW-M2's `ViewModelFirmwareVerifier`, which also compares versions in Swift.
- **Core today:** `DspState.platformId` and `fwMajor` to `fwBeta`, which `CoreMirror.applyCoreIdentity`
  (`:909-921`) already uses, and `firmwareVersionMatches`. The core's `fetchAll` does its own connect reads.
- **Proposed: SWS8, "Identity from the core" (Swift, 0.25).** With a core attached, skip `fetchPlatform` and
  `waitForDeviceReady`: the core's connect is the readiness probe. Take the match from `firmwareVersionMatches`.
  `FirmwareVersion` stays for the app's own version strings (update checker, What's New, onboarding).

### M9. Pin rules

- **Evidence:**
  - `DSPViewModel.swift:2453-2577` (`pinInUseBy` and `pinAssignment`: the firmware's reservation rules for every
    pin consumer);
  - `DSPi_ConsoleApp.swift:8593-8597` (`validPins`, "mirrors firmware `is_valid_gpio_pin()`");
  - `:8607-8640` (`i2sAllowedFor`, the STM32H723 SAI slot dependencies);
  - `:8582` (the PDM pin index by platform).
- **Core today:** `csPinUses()` (`cs.rs:724`) covers control-surface pins. `dspi-session/src/pins.rs:90-94`
  (`valid_pins`) and the TUI's `pin_owner` (`dspi-tui/src/settings/cs_model.rs:25`) are not exported. Qt
  (`apps/desktop/qt/src/controllers/PinMap.cpp`) and Android (`DspiBridgeImpl.kt:1133`) each have their own copy.
- **Proposed: SWS9, "Pin map in the core" (a CG of about 0.75 plus Swift 0.25).** A core export answers "who holds
  pin N, excluding consumer C" and lists the valid pins per platform over `DspState`. Then the three clients drop
  their copies.

### M10. Link discovery details

- **Evidence:**
  - `Link/HubBrowser.swift:243-262` (`parseAddress`: host, port, IPv6 brackets, the default port; also used by
    `NetworkingSettingsView.swift:314` and `LinkClientCoordinator.swift:82`);
  - `:283-300` (decodes the `/info` JSON for a typed address);
  - `:583-597` (`hub(fromTXT:)`, the TXT keys of spec 3.1).
- **Core today:** `linkParseAddress(text:)` and `linkFetchInfo(host:port:)` are exported, and Swift does not use
  either. The core's TXT decoding (`dspi_link::discovery`) sits behind the `mdns` feature, which the mobile
  libraries leave off (`dspi-core/src/link_browse.rs:1-12`). It is not exported.
- **Proposed: SWS10, "Hub records from the core" (Swift 0.25, plus a CG of about 0.25 that exports a TXT-map
  parser without `mdns`).** `NWBrowser` and the permissions stay Swift.

### M11. The Mac's AutoEQ rebuild

- **Evidence:** `AutoEQ/AutoEQManager.swift:404-729`: `rebuildDatabase` walks GitHub's listings, downloads the
  profiles and parses them (`parseProfile`, `:622`). The Mac's menu calls it (`DSPi_ConsoleApp.swift:11726-11887`,
  `AutoEQMenuActions`, `:11785`). iOS already uses the core (`AutoEQ/AutoEQCoreUpdate.swift`, whose header says
  "The Mac's menu keeps its own Swift rebuild for now").
- **Core today:** CG6's `autoeqRebuildStart`, `autoeqImport`, `autoeqResetToBuiltin` and `autoeqUpdate`.
- **Proposed: SWS11, "Mac AutoEQ update through CG6" (Swift, 0.25).** The Mac's menu calls
  `AutoEQCoreUpdate.swift`, and the Swift rebuild goes.

### M12. Smaller items (one WP, SWS12, Swift 0.5, plus a CG one-liner for a)

- **a.** `ExtensionChannel.swift:272-300` (`VregVoltage`: the pico-sdk code-to-volts table) is used on the core path
  by `CoreVoltageStatusView.swift:49`. `dspi_proto::ext::vreg_mv` and `vreg_label` (`dspi-proto/src/ext.rs:277-283`)
  exist but are not exported. Note: the Swift table is per platform (on RP2040 codes 0-5 read 0.80 V) and the
  proto one is not. Settle that in the core.
- **b.** `ExtensionChannel.swift:147-185` (`CrossfeedPairMap.validated`, `isValid`), used by
  `CrossfeedPairMapView.swift:61` on the core path. The core exports `crossfeedPairMapValid`.
- **c.** `DSPi_ConsoleApp.swift:10774` (Mac) and `:11537` (iOS): a configuration import decodes the file with
  Swift's `PresetDocumentFile.decode` (`PresetDocument.swift:721-747`) before handing it to the core's preview,
  only to word the prompt. The core's `configInspect` and `configPreview` can word it. After that,
  `PresetDocument.swift` serves only the opt-out.
- **d.** `SpectrumAnalyser.swift:1161-1188`: the core's dB snapshot goes back into wire level bytes
  (`RTA_LEVEL_STEP_DB`) so that the renderers keep their byte model. This is an adapter, not protocol. It goes
  once the Metal renderers take dB.

### SWS13. Retire the opt-out (after SW-M3 and a release on the core)

Section 3.2's list: the legacy files, the raw branches, the wire structs, the Swift bulk decoder and mirror, and
the Swift demo. Also `Constants.swift`'s opcodes, with its remaining shared names moved to `DspiConstants`. This
is the step that empties the grep in section 5 together with SWS7 and SW-M3. It is an owner decision, because
`-CoreTransport NO` is the fallback during the transition (SW-M1).

## 5. The done check: opcodes outside `Core/`

```
cd "apps/apple/DSPi Console"
grep -rnE '\bREQ_[A-Z0-9_]+' --include=*.swift . | grep -v '/Generated/' | grep -vE '^(\./)?Core/'
```

On `ab475933` this finds **805 lines in 30 files**. Of these, 635 are not comments. By file, non-comment lines:

| Lines | File | Goes with |
|---:|---|---|
| 240 | `Constants.swift` | SWS13 |
| 229 | `Commands.swift` | SWS13 (raw branches); M7's `typedFloatSet` cases with SWS7 |
| 87 | `Demo/FakeTransport.swift` | SWS13 |
| 28 | `StatsView.swift` | SWS13 |
| 9 | `SpectrumAnalyser.swift` | SWS13 |
| 7 | `MIDI/LegacyMIDITransport.swift` | SWS13 |
| 7 + 5 + 3 + 2 + 2 | `SubharmonicSynthView`, `PsychoacousticBassView`, `VolumeLevellerView`, `LoudnessView`, `CrossfeedView` | **SWS7** |
| 4 + 1 | `ExtensionCommands.swift`, `ExtensionChannel.swift` | SWS13 |
| 4 + 3 + 2 + 1 + 1 | `Link/CommandRouter`, `USBHubDevice`, `LinkHub`, `CoreVoltageRule`, `NetworkTransport` | SW-M3, SWS13 |

The other 170 lines are comments that name an opcode, in UI and model files (for example `DSPViewModel.swift:95`,
`ContentView.swift:863`). A strict "empty" needs either a comment-aware grep, which is the one above with
`| grep -vE '^[^:]+:[0-9]+:\s*(//|///|\*)'` added, or comments that stop naming opcodes. Inside `Core/`, the only
opcodes are `HostBridge`'s `REQ_GET_ALL_PARAMS` and `REQ_SET_ALL_PARAMS` (`Core/CoreBackedTransport.swift:1394`,
`:1432`), on the Swift hub's local session until SW-M3.

After SWS7 the views are clean. After SW-M3 and SWS13, what is left is the core's own constants module, which the
grep does not cover (`Core/Generated/`).

## 6. Every file

**UI (68).**
- `AboutLicence`, `ChannelPalette`, `Components`, `ContentView`, `CoreVoltageStatusView` (M12a), `CrossfeedPairMapView`
  (M12b), `CrossfeedView` (M7), `CustomSlider`, `DashboardView`, `GraphSpectrumOverlay`, `GraphView`,
  `LiveGraphHost`, `LoudnessView` (M7), `MatrixMixerView`, `Models`, `OutputLimiterView`, `PeqBandHUD`,
  `PeqGraphEditor` (M5), `PeqGraphModel` (M5), `PeqGraphSupport`, `PsychoacousticBassView` (M7),
  `SpectrumAnalyserView`, `StatsView` (raw opt-out branch, SWS13), `SubharmonicSynthView` (M7), `TestSignalsView`,
  `ToolParameters`, `TubeIllustration`, `TubeModellerView`, `UpmixerView`, `USBDevice` (device model),
  `VolumeLevellerView` (M7).
- `AutoEQ/AutoEQBrowser`, `AutoEQ/AutoEQPreview`, `Installer/InstallerViews`, `Snapshots/SnapshotsFiles`,
  `Snapshots/SnapshotsView`.
- `FirmwareInstallUI`, `FirmwareUpdateView`, `Onboarding/GettingStartedView` (all three changing in SW-M2).
- `NetworkingSettingsView` (changing in SW-M3).
- `Core/`: `CoreBackedTransport` (`HostBridge` changing in SW-M3), `CoreControlSurfaces`, `CoreDiagnostics`,
  `CoreInstaller`, `CoreLink`, `CoreSnapshots`, `CoreUndo`, `NoticeCenter`, `OutputLabels`, `ReadOnlyFloor`,
  `UserConfirmation`.
- `Link/CompositeTransport`, `Link/DeviceTransport`, `Link/PairingPrompt`, `MIDI/LocalTransportSet`.
- `iOS/IOSApp`, `iOS/IOSAutoEQMenu`, `iOS/iOSCompat`, `iOS/IOSLinkPairing`, `iOS/PeqTouchEditor` (M5).
- `Onboarding/CoachMarkPanel`, `CoachMark`, `MainWindowRoot`, `OnboardingCatalogue`, `OnboardingCoordinator`,
  `OnboardingDebug`, `OnboardingHint`, `WhatsNewView`.

  The onboarding text is Swift's own. Qt reads the core's `onboarding_getting_started_json`. Sharing that content
  is a product choice, not protocol.

**OS integration (27).**
- `AppIdentity`, `AppUpdateChecker`, `AppUpdateInstaller`, `HostAudioFormatMonitor`, `MenuBarMode`, `TestHostGuard`.
- `BootloaderLocator` (changing in SW-M2).
- `AutoEQ/AutoEQCoreUpdate`, `Core/CoreUsb`, `iOS/DocumentPicker`.
- `Link/LinkClientCoordinator`, `Link/LinkTokenStore`, `Link/LinkDiscovery` and `Link/LinkService` (both changing in
  SW-M3).
- `MIDI/CoreMidiPipe`, `MIDI/MIDIControlPort`, `MIDI/MIDIRelayPort`, `MIDI/MIDITransport`.
- `PeqGraphRenderer`, `PeqGraphShaders`, `RtaBarShaders`, `RtaCurveShaders`, `RtaMetalBars`, `RtaMetalCurves`,
  `RtaMetalSurface`, `RtaRenderCache`.
- `../Tools/UpdateSigning/update-signing.swift`.

**Opt-out legacy (27).**
- `Constants`, `Demo/FakeTransport`, `ExtensionCommands`, `PresetSnapshot`, `SpectrumAnalyser`.
- `Link/LinkClient`, `Link/LinkClientProtocol`, `Link/NetworkTransport`, `MIDI/LegacyMIDITransport`,
  `MIDI/MIDIControlCodec`.
- Changing in SW-M3: `Link/AuthStore`, `CommandRouter`, `CoreVoltageRule`, `DeviceRegistry`, `HubDevice`,
  `HubTransport`, `LinkFrames`, `LinkHub`, `LinkMessages`, `LinkPolicy`, `LinkRole`, `LinkServer`,
  `LinkSessionHandler`, `NotifyRelay`, `PollScheduler`, `SnapshotCache`, `USBHubDevice`.

**Should be in the core (14).**

| File | Items |
|---|---|
| `AutoEQ/AutoEQManager` | M11; the rest is the database file and favourites (UI) and the opt-out's raw apply |
| `Commands` | M6, M7, M8; the rest is opt-out branches |
| `Core/CoreMirror` | M2, M3 |
| `DSPi_ConsoleApp` | M4, M9, M11, M12c; the rest is UI |
| `DSPMath` | M5 |
| `DSPViewModel` | M1, M2, M5, M8, M9; plus the opt-out wire structs |
| `ExtensionChannel` | M12a, M12b; plus the opt-out wire formats |
| `FilterFile` | M4 |
| `FirmwareInstaller` | Changing in SW-M2 |
| `FirmwareVersion` | M8 |
| `InterruptMonitor` | M1; plus the opt-out decoder |
| `Link/HubBrowser` | M10; the `NWBrowser` part is OS integration |
| `PresetDocument` | M12c |
| `PresetDocumentTransfer` | M3 (`customCrossfeedPair`); the rest is the opt-out's capture and apply |

## 7. How this was made

- **Signals per file:** a script counts, for each file, the raw transfer calls (`sendControlRequest`,
  `getControlResult`, `getControlRequest`, `rawGet`, `rawSet`), the `REQ_` references, the byte arithmetic, the
  SwiftUI views, the core references and the imports.
- **Raw transfers:** a second script lists every function with a raw transfer and whether it checks for a core
  (`funcs.py`). Each unguarded one was traced to its callers.
- **Decoders and duplicates:** every `fromData`, `toData`, `parse` and `decode` was traced to its callers. The
  rest came from reading the code: wire-version and platform-name comparisons, `bulkRaw` reads, notification
  handlers, and core exports that Swift does not call.
- **Scratch:** the scripts and their output are in `~/.cache/dspi-build/work/wp-SWM4/` (`signals.sh`,
  `signals.txt`, `funcs.py`, `rawfuncs.txt`, `classes.tsv`).

Nothing was built or run. This WP changes no code.
