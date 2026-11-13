import XCTest
import dspi_core
import dspi_constants
@testable import DSPi_Console

/// WP SW11: pins, I2S, ADAT, inputs, UART/I2C, LG Sound Sync, DAC mute and
/// names through the core's typed API (Commands.swift, ExtensionCommands.swift).
/// The edits go through the view model as the views make them; Undo and Redo
/// step back what the core's registry calls undoable (channel names, LG Sound
/// Sync, the master clock) and step over the rest (the flash-written names,
/// the pins, buses and inputs), as in Qt.
final class CoreDomainSW11Tests: XCTestCase, CoreDomainTesting {

    /// `a`, or `b` when the setting already holds `a`: an edit that changes it.
    private func other<T: Equatable>(_ current: T, _ a: T, _ b: T) -> T { current == a ? b : a }

    /// `body` off main, as the views call the setters that wait for a status.
    private func offMain<T>(_ body: () -> T) -> T { DispatchQueue.global().sync(execute: body) }

    /// Every request this domain has on the wire, SET and GET.
    private static let domainRequests: Set<UInt8> = [
        REQ_SET_LG_SOUND_SYNC_ENABLE, REQ_GET_LG_SOUND_SYNC_ENABLE, REQ_SET_DAC_HW_MUTE_CONFIG,
        REQ_GET_DAC_HW_MUTE_CONFIG, REQ_TEST_DAC_HW_MUTE, REQ_SET_OUTPUT_PIN, REQ_GET_OUTPUT_PIN,
        REQ_SET_OUTPUT_TYPE, REQ_GET_OUTPUT_TYPE, REQ_SET_I2S_BCK_PIN, REQ_GET_I2S_BCK_PIN, REQ_SET_MCK_ENABLE,
        REQ_GET_MCK_ENABLE, REQ_SET_MCK_PIN, REQ_GET_MCK_PIN, REQ_SET_MCK_MULTIPLIER, REQ_GET_MCK_MULTIPLIER,
        REQ_GET_ADAT_STATUS, REQ_SET_ADAT_ENABLE, REQ_SET_ADAT_PIN, REQ_GET_ADAT_INPUT_STATUS,
        REQ_SET_ADAT_INPUT_ENABLE, REQ_SET_ADAT_INPUT_PIN, REQ_SET_ADAT_INPUT_CLOCK_MODE, REQ_GET_INPUT_SOURCE,
        REQ_SET_INPUT_SOURCE, REQ_GET_INPUT_SOURCE_NAME, REQ_SET_INPUT_SOURCE_NAME, REQ_GET_DEVICE_NAME,
        REQ_SET_DEVICE_NAME, REQ_GET_SPDIF_INPUT_CONFIG, REQ_GET_SPDIF_RX_PIN, REQ_SET_SPDIF_RX_PIN,
        REQ_SET_SPDIF_INPUT_ENABLE, REQ_GET_I2S_RX_PIN, REQ_SET_I2S_RX_PIN, REQ_GET_I2S_INPUT_CHANNELS,
        REQ_SET_I2S_INPUT_CHANNELS, REQ_GET_INPUT_RATE, REQ_SET_INPUT_RATE, REQ_GET_I2S_CLOCK_MODE,
        REQ_SET_I2S_CLOCK_MODE, REQ_GET_I2S_SLAVE_STATUS, REQ_GET_I2S_CLOCK_PIN_MODE, REQ_SET_I2S_CLOCK_PIN_MODE,
        REQ_GET_CTRL_IFACE_STATUS, REQ_GET_UART_CONFIG, REQ_GET_I2C_CONFIG, REQ_SET_UART_CONFIG,
        REQ_SET_I2C_CONFIG, REQ_SET_CHANNEL_NAME, REQ_GET_CHANNEL_NAME, REQ_EXTENSION,
    ]

    private func domainRaw(_ t: CoreBackedTransport) -> [String] {
        t.takeRawOpcodesForTest().filter { Self.domainRequests.contains($0) }.map { String(format: "0x%02X", $0) }
    }

    // MARK: - Undo and Redo

    func testSW11UndoAndRedoAChannelName() {
        let (core, t, vm) = demoVM()
        XCTAssertFalse(vm.undo.canUndo, "a new connection has nothing to undo")
        let name = vm.channelNames[0]
        let to = other(name, "SW11 left", "SW11 main")
        vm.setChannelName(channel: 0, name: to)
        XCTAssertEqual(entries(core, t, "ch.name").count, 1, "journalled")
        undoAndRedo(vm, undone: { vm.channelNames[0] == name }, redone: { vm.channelNames[0] == to })
    }

    func testSW11UndoAndRedoLgSoundSync() {
        let (_, _, vm) = demoVM()
        let on = vm.lgSoundSyncEnabled
        vm.setLgSoundSyncEnabled(!on)
        undoAndRedo(vm, undone: { vm.lgSoundSyncEnabled == on }, redone: { vm.lgSoundSyncEnabled == !on })
    }

    /// The master clock is hardware, but the registry calls it harmless: its
    /// setters wait for the device's status, and Undo puts it back.
    func testSW11UndoAndRedoTheMasterClock() {
        let (core, t, vm) = demoVM()
        let mult = vm.mckMultiplier
        let to = other(mult, 256, 128)
        XCTAssertEqual(offMain { vm.setMckMultiplier(to) }, PIN_CONFIG_SUCCESS)
        XCTAssertEqual(entries(core, t, "i2s.mck.mult").count, 1, "journalled")
        undoAndRedo(vm, undone: { vm.mckMultiplier == mult }, redone: { vm.mckMultiplier == to })

        let on = vm.mckEnabled
        XCTAssertEqual(offMain { vm.setMckEnable(!on) }, PIN_CONFIG_SUCCESS)
        undoAndRedo(vm, undone: { vm.mckEnabled == on }, redone: { vm.mckEnabled == !on })
    }

    /// A source name and the device name write flash, and a source switch or
    /// an output's bus reconfigures the I/O: the core records them, and Undo
    /// steps over them (saying so) to the change before.
    func testSW11FlashAndReconfigurationEditsAreSteppedOver() {
        let (core, t, vm) = demoVM()
        let lg = vm.lgSoundSyncEnabled
        vm.setLgSoundSyncEnabled(!lg)
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo })
        XCTAssertTrue(spinUntil(5) { vm.inputSourceNamesSupported && vm.deviceNameSupported },
                      "the demo device names its sources and itself")
        vm.setInputSourceName(source: 2, name: "SW11 deck")
        vm.setDeviceName("SW11 den")
        let type = vm.outputSlotTypes[1]
        XCTAssertEqual(offMain { vm.setOutputSlotType(slot: 1, type: type == 0 ? 1 : 0) }, PIN_CONFIG_SUCCESS)
        for path in ["in.name", "dev.name", "out.type"] {
            XCTAssertEqual(entries(core, t, path).count, 1, "\(path) recorded")
        }

        vm.undo.undo()
        XCTAssertTrue(spinUntil(5) { vm.lgSoundSyncEnabled == lg }, "the LG change is undone")
        XCTAssertNotNil(vm.undo.notice, "the steps over are named")
        XCTAssertEqual(vm.inputSourceNames[2], "SW11 deck", "a flash write stays")
        XCTAssertEqual(vm.deviceName, "SW11 den")
        XCTAssertEqual(vm.outputSlotTypes[1], type == 0 ? 1 : 0, "a reconfiguration stays")
    }

    // MARK: - No raw requests in this domain

    /// Every setter and read of the domain, on the core's demo device: no
    /// request of the domain goes out raw (each is a typed call), the journal
    /// holds no raw entry, and a fresh read of the device finds the values.
    func testSW11EveryDomainCallGoesThroughTheCore() {
        let (core, t, vm) = demoVM()
        _ = t.takeRawOpcodesForTest()   // the connect-time read

        let lg = !vm.lgSoundSyncEnabled
        let source = other(vm.inputSource, 1, 0)
        let mult = other(vm.mckMultiplier, 256, 128)
        let rate = other(vm.i2sInputRateHz, UInt32(96_000), 44_100)
        let clockMode = other(vm.i2sClockMode, I2S_CLOCK_MODE_SLAVE, I2S_CLOCK_MODE_MASTER)
        vm.setLgSoundSyncEnabled(lg)
        vm.setChannelName(channel: 1, name: "SW11 right")
        vm.setInputSourceName(source: 1, name: "SW11 optical")
        vm.setDeviceName("SW11 unit")
        vm.setInputSource(source)
        vm.setInputRate(rate)
        vm.setI2SClockMode(clockMode)
        vm.setDacHwMuteConfig(DacHwMuteConfig(enabled: false, activeLow: true, pin: 22, holdMs: 20, releaseMs: 40))
        let statuses: [String: UInt8] = offMain {
            var s: [String: UInt8] = [:]
            s["mck.mult"] = vm.setMckMultiplier(mult)
            s["mck.on"] = vm.setMckEnable(vm.mckEnabled)
            s["mck.pin"] = vm.setMckPin(vm.mckPin)
            s["out.pin"] = vm.setOutputPin(output: 0, pin: vm.outputPins[0])
            s["out.type"] = vm.setOutputSlotType(slot: 1, type: vm.outputSlotTypes[1])
            s["bck"] = vm.setI2SBckPin(vm.i2sBckPin)
            s["clockpins"] = vm.setI2SClockPinMode(vm.i2sClockPinMode)
            s["adat.on"] = vm.setAdatEnable(vm.adatEnabled)
            s["adat.pin"] = vm.setAdatPin(vm.adatPin)
            s["adatin.on"] = vm.setAdatInputEnable(vm.adatInputEnabled)
            s["adatin.pin"] = vm.setAdatInputPin(vm.adatInputPin)
            s["adatin.clock"] = vm.setAdatInputClockMode(vm.adatInputClockMode)
            s["spdif.pin"] = vm.setSpdifRxPin(index: 0, vm.spdifRxPin)
            s["spdif.on"] = vm.setSpdifInputEnable(index: 1, vm.spdifExtEnabled[0])
            s["i2s.ch"] = vm.setI2SInputChannels(vm.i2sInputChannels)
            s["i2s.pin"] = vm.setI2SRxPin(pair: 0, vm.i2sRxPins[0])
            s["dac.test"] = vm.testDacHwMute()
            _ = vm.setUartCtrlConfig(vm.uartCtrlConfig)
            _ = vm.setI2cCtrlConfig(vm.i2cCtrlConfig)
            _ = vm.assignOutputPin(output: 1, pin: vm.outputPins[1])
            // The reads.
            vm.fetchLgSoundSyncEnabled()
            vm.fetchDacHwMuteConfig()
            vm.fetchOutputPin(output: 0)
            vm.fetchOutputSlotType(slot: 0)
            vm.refreshOutputSlotTypes()
            vm.fetchI2SBckPin()
            vm.fetchMckEnable()
            vm.fetchMckPin()
            vm.fetchMckMultiplier()
            vm.fetchAdatConfig()
            vm.fetchAdatInputConfig()
            vm.fetchInputSource()
            vm.fetchSpdifRxPin(index: 0)
            vm.fetchI2SInputChannels()
            vm.fetchInputRate()
            vm.fetchControlInterfaces()
            vm.fetchCtrlIfaceStatus()
            vm.fetchChannelName(channel: 1)
            vm.probeExtensions(force: true)
            vm.refreshCoreVregStatus()
            return s
        }
        flush(t)
        settle()

        XCTAssertEqual(domainRaw(t), [], "no request of the domain went out raw")
        XCTAssertTrue(entries(core, t, "raw").isEmpty, "no raw write in the journal")
        XCTAssertEqual(statuses["mck.mult"], PIN_CONFIG_SUCCESS)
        for (name, status) in statuses {
            XCTAssertNotEqual(status, 0xFF, "\(name) has the device's answer")
        }
        for path in ["in.lg", "ch.name", "in.name", "dev.name", "in.source", "in.rate", "in.i2s.clock", "i2s.mck.mult"] {
            XCTAssertFalse(entries(core, t, path).isEmpty, "\(path) went through its typed setter")
        }

        // A fresh read of the demo device, through the core.
        guard case .success(let s) = t.readAll() else { return XCTFail("the device reads") }
        XCTAssertEqual(s.lgSoundSyncEnabled, lg)
        XCTAssertEqual(s.channelNames[safe: 1], "SW11 right")
        XCTAssertEqual(Int(s.inputSource), source)
        XCTAssertEqual(i2sRateEnumToHz(s.i2sInputRate), rate)
        XCTAssertEqual(s.i2sClockMode, clockMode)
        XCTAssertEqual(s.mckMultiplier == 1 ? 256 : 128, mult)
        XCTAssertEqual(try t.call { try $0.getDeviceName() }.get(), "SW11 unit")
        XCTAssertEqual(try t.call { try $0.getInputSourceName(source: 1) }.get(), "SW11 optical")
        // And the view model shows them.
        XCTAssertTrue(spinUntil(5) { vm.channelNames[1] == "SW11 right" && vm.deviceName == "SW11 unit" })
        XCTAssertEqual(domainRaw(t), [], "nor did the reads after")
    }

    /// The same over Swift's FakeTransport under the core: the OUT setters
    /// reach the device's wire (the fake's image and echo store).  The fake
    /// does not answer the write-as-read pin setters; the demo device test
    /// above covers those.
    func testSW11TheOutSettersReachTheDevice() {
        let (fake, core, t, vm) = hostVM()
        _ = t.takeRawOpcodesForTest()
        let lg = !vm.lgSoundSyncEnabled
        vm.setLgSoundSyncEnabled(lg)
        vm.setChannelName(channel: 2, name: "SW11 sub")
        vm.setInputSource(1)
        flush(t)
        settle()
        XCTAssertEqual(domainRaw(t), [])
        XCTAssertTrue(entries(core, t, "raw").isEmpty)
        let get = { (request: UInt8, value: UInt16, length: UInt16) in
            fake.getControlRequest(request: request, value: value, index: HostBridge.vendorInterface, length: length)
        }
        XCTAssertEqual(get(REQ_GET_LG_SOUND_SYNC_ENABLE, 0, 1)?.first, lg ? 1 : 0)
        XCTAssertEqual(get(REQ_GET_INPUT_SOURCE, 0, 1)?.first, 1)
        let name = get(REQ_GET_ALL_PARAMS, 0, UInt16(BULK_PARAMS_SIZE)).map {
            String(decoding: $0[(BULK_CHANNEL_NAMES_OFFSET + 64)..<(BULK_CHANNEL_NAMES_OFFSET + 96)].prefix { $0 != 0 },
                   as: UTF8.self)
        }
        XCTAssertEqual(name, "SW11 sub")
    }

    /// A refused pin keeps the device's status (the core's `Refused`), as the
    /// raw request returned it, and the view keeps the device's pin.
    func testSW11ARefusedPinReturnsTheDevicesStatus() {
        let (_, _, vm) = demoVM()
        let before = vm.mckMultiplier
        // Seven is no multiplier: the device answers INVALID_PIN.
        XCTAssertEqual(DSPViewModel.pinStatus(.failure(.Refused(status: PIN_CONFIG_INVALID_PIN))), PIN_CONFIG_INVALID_PIN)
        XCTAssertEqual(DSPViewModel.pinStatus(.failure(.Disconnected)), 0xFF)
        XCTAssertEqual(DSPViewModel.pinStatus(.success(())), PIN_CONFIG_SUCCESS)
        let enabled = offMain { vm.setAdatInputEnable(true) }
        if vm.adatInputPin == ADAT_INPUT_PIN_UNSET || vm.adatInputPin == 0 {
            XCTAssertEqual(enabled, PIN_CONFIG_INVALID_PIN, "no ADAT input pin set")
            XCTAssertFalse(vm.adatInputEnabled)
        }
        XCTAssertEqual(vm.mckMultiplier, before)
    }

    // MARK: - Status events and reads through the core

    /// ADAT out, the I2S slave and ADAT input locks: read through the core
    /// (its typed status and lock reads), and again on a stream event (CG25).
    func testSW11StreamStatesThroughTheCore() {
        let (_, t, vm) = demoVM()
        _ = t.takeRawOpcodesForTest()
        offMain {
            vm.fetchAdatStatus()
            vm.fetchI2SSlaveStatus()
            vm.fetchAdatInputStatus()
        }
        settle()
        XCTAssertEqual(domainRaw(t), [], "status reads are typed")
        XCTAssertEqual(vm.adatSupported, vm.platformName == "RP2350")

        // A stream event re-reads them (no raw request either).
        vm.adatStatus.resyncCount = 0xBEEF
        vm.followCoreEvents(DspiConstants.NOTIFY_STREAMS)
        if vm.adatSupported {
            XCTAssertTrue(spinUntil(5) { vm.adatStatus.resyncCount != 0xBEEF }, "ADAT out read again")
        }
        settle()
        XCTAssertEqual(domainRaw(t), [])
    }

    /// Another client's names arrive as the core's NOTIFY_NAMES: the view
    /// model reads the core's copy, with no request.
    func testSW11NamesFollowTheCoresCopy() {
        let (core, t, vm) = demoVM()
        XCTAssertTrue(spinUntil(5) { vm.inputSourceNamesSupported })
        _ = t.takeRawOpcodesForTest()
        t.coreQueue.sync { try? core.demoRenameElsewhere(target: 1, name: "Elsewhere") }
        XCTAssertTrue(spinUntil(5) { vm.inputSourceNames[1] == "Elsewhere" },
                      "shown: \(vm.inputSourceNames[safe: 1] ?? "none")")
        XCTAssertEqual(domainRaw(t), [])
    }

    /// The extension probe and the core voltage come from the core (no
    /// request), and this app writes no voltage.
    func testSW11ExtensionsAndCoreVoltageThroughTheCore() {
        let (_, t, vm) = hostVM()   // the fake answers like the bofr fork
        _ = t.takeRawOpcodesForTest()
        offMain { vm.probeExtensions(force: true) }
        XCTAssertTrue(spinUntil(5) { vm.extensionProbe != .unknown })
        if vm.coreVregSupported {
            XCTAssertTrue(spinUntil(5) { vm.coreVregStatus != nil }, "the core's read")
        }
        settle()
        XCTAssertEqual(domainRaw(t), [], "EXT_LIST and EXT_CORE_VREG are the core's")
    }
}
