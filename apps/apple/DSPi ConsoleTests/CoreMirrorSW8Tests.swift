import XCTest
import Combine
import dspi_core
import dspi_constants
@testable import DSPi_Console

/// WP SW8: the view model's state from the Rust core's `DspState`
/// (Core/CoreMirror.swift), whatever the launch flag says: every test builds
/// its own core transport.
final class CoreMirrorSW8Tests: XCTestCase {

    // MARK: - Helpers

    /// A view model over `transport`, after its connect-time read finished.
    private func connected(_ transport: any DeviceTransport, select: () -> Void,
                           file: StaticString = #filePath, line: UInt = #line) -> DSPViewModel {
        let vm = DSPViewModel(transport: transport)
        select()
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == transport.generation },
                      "the connect-time read finishes", file: file, line: line)
        settle()
        return vm
    }

    /// The core's demo device and a view model on it.
    private func demoVM() -> (CoreBackedTransport, DSPViewModel) {
        let t = CoreBackedTransport(link: .demo)
        let vm = connected(t) { t.selectDevice(FakeTransport.demoDevice) }
        return (t, vm)
    }

    /// Let main-queue publications from background work land.
    private func settle(_ seconds: TimeInterval = 0.3) {
        RunLoop.current.run(until: Date().addingTimeInterval(seconds))
    }

    private static func float(_ value: Float) -> Data { withUnsafeBytes(of: value) { Data($0) } }

    /// One GET behind every earlier SET on the transport's queue.
    private func flush(_ t: any DeviceTransport) {
        _ = t.getControlResult(request: REQ_GET_PLATFORM, value: 0, index: 2, length: 7)
    }

    // MARK: - Field by field against the Swift decoder (SWIFT-SEAMS 8.3)

    /// Every value the Swift bulk decoder writes, as text, so two view models
    /// compare field by field (FilterParams' `id` left out).
    private static func fields(_ vm: DSPViewModel) -> [String: String] {
        func bands(_ d: [Int: [DSPi_Console.FilterParams]]) -> String {
            d.keys.sorted().map { ch in
                "\(ch):" + (d[ch] ?? []).map { "\($0.type.rawValue)/\($0.freq)/\($0.q)/\($0.gain)/\($0.bypass)/\($0.qp)/\($0.active)" }
                    .joined(separator: ",")
            }.joined(separator: " ")
        }
        let l = vm.loudness, c = vm.crossfeed, p = vm.psybass, s = vm.subharm, t = vm.tube, u = vm.upmix, v = vm.leveller
        var f: [String: String] = [
            "platformName": vm.platformName,
            "numInputChannels": "\(vm.numInputChannels)",
            "preampDB": "\(vm.preampDB)",
            "masterVolumeDB": "\(vm.masterVolumeDB)",
            "userVolumeDB": "\(vm.userVolumeDB)",
            "bypass": "\(vm.bypass)",
            "loudness": "\(vm.loudnessEnabled) \(l.outputMask) \(l.refSPL) \(l.intensity)",
            "crossfeed": "\(vm.crossfeedEnabled) \(c.preset) \(c.itd) \(c.freq) \(c.feed) \(c.outputMask) \(c.pairMap.bytes)",
            "psybass": "\(vm.psybassEnabled) \(p.outputMask) \(p.cutoffHz) \(p.harmonicsDB) \(p.driveDB) \(p.characterPct) \(p.originalDB)",
            "subharm": "\(vm.subharmEnabled) \(s.outputMask) \(s.lowDB) \(s.highDB) \(s.topDB) \(s.boostDB) \(s.selectMode) "
                + "\(s.selectDepthPct) \(s.selectHoldMs) \(s.ceilingDB) \(s.linkPairs)",
            "tube": "\(vm.tubeEnabled) \(t.outputMask) \(t.type) \(t.driveDB) \(t.biasPct) \(t.asymDB) \(t.hardnessPct) "
                + "\(t.sagPct) \(t.rectifier) \(t.xfmrEnabled) \(t.xfmrDamping) \(t.xfmrResHz) \(t.mixPct) \(t.trimDB)",
            "limiter": vm.limiter.outputs.map { "\($0.enabled)/\($0.thresholdDB)/\($0.releaseMs)/\($0.linkGroup)" }
                .joined(separator: ","),
            "upmix": "\(vm.upmixEnabled) \(u.centerMode) \(u.surroundMode) \(u.presenceDB) \(u.strengthPct) \(u.centerWidthPct) "
                + "\(u.thresholdPct) \(u.attackMs) \(u.releaseMs) \(u.detectorHpfHz) \(u.surroundDelayMs) "
                + "\(u.surroundHpfHz) \(u.surroundLpfHz) \(u.decorrPct)",
            "leveller": "\(vm.levellerEnabled) \(v.amount) \(v.speed) \(v.maxGainDB) \(v.lookahead) \(v.gateDB) "
                + "\(v.detectorMask) \(v.applyMask)",
            "channelDelays": vm.channelDelays.keys.sorted().map { "\($0):\(vm.channelDelays[$0]!)" }.joined(separator: ","),
            "matrixRouting": "\(vm.matrixRouting)",
            "matrixGain": "\(vm.matrixGain)",
            "matrixInvert": "\(vm.matrixInvert)",
            "outputEnabled": "\(vm.outputEnabled)",
            "outputMuted": "\(vm.outputMuted)",
            "outputGainDB": "\(vm.outputGainDB)",
            "outputDelayMS": "\(vm.outputDelayMS)",
            "outputPins": "\(vm.outputPins)",
            "outputSlotTypes": "\(vm.outputSlotTypes)",
            "i2s": "\(vm.i2sBckPin) \(vm.mckPin) \(vm.mckEnabled) \(vm.mckMultiplier) \(vm.i2sClockPinModeSupported) "
                + "\(vm.i2sClockPinMode) \(vm.i2sBckPinSlave)",
            "channelData": bands(vm.channelData),
            "xoverData": bands(vm.xoverData),
            "channelNames": "\(vm.channelNames)",
            "input": "\(vm.inputSource) \(vm.inputSourceSupported) \(vm.spdifRxPin) \(vm.multiSpdifSupported) "
                + "\(vm.spdifInputCount) \(vm.spdifExtEnabled) \(vm.spdifRxPinsExt)",
            "i2sInput": "\(vm.i2sInputSupported) \(vm.i2sRxPins) \(vm.i2sInputChannels) \(vm.i2sInputRateHz) "
                + "\(vm.i2sClockModeSupported) \(vm.i2sClockMode) \(vm.i2sSlaveStatus.clockMode)",
            "lg": "\(vm.lgSoundSyncEnabled) \(vm.lgSoundSyncSupported)",
            "dacHwMute": "\(vm.dacHwMuteConfig) \(vm.dacHwMuteSupported)",
            "adat": "\(vm.adatSupported) \(vm.adatEnabled) \(vm.adatPin) \(vm.adatStatus.enabled) \(vm.adatStatus.pin)",
            "adatInput": "\(vm.adatInputSupported) \(vm.adatInputPin) \(vm.adatInputEnabled) \(vm.adatInputClockMode) "
                + "\(vm.adatInputStatus.pin) \(vm.adatInputStatus.enabled) \(vm.adatInputStatus.clockMode)",
            "wire": "\(vm.firmwareWireFormatVersion) \(vm.firmwareSupportsCrossover)",
            "presets": "\(vm.presetOccupied) \(vm.presetNames) \(vm.activePresetSlot)",
        ]
        if let v = vm.firmwareVersion { f["firmwareVersion"] = "\(v.major).\(v.minor).\(v.patch)-\(v.beta)" }
        return f
    }

    /// The captured image with a non-default value in every section the
    /// decoder reads, the +1-encoded presence bytes set, and a limiter group
    /// that disagrees (both sides gang it).
    static func busyImage(_ base: Data) -> Data {
        var d = base
        func f(_ off: Int, _ v: Float) { d.replaceSubrange(off..<(off + 4), with: float(v)) }
        func u16(_ off: Int, _ v: UInt16) { d[off] = UInt8(v & 0xFF); d[off + 1] = UInt8(v >> 8) }
        func band(_ base: Int, _ type: FilterType, _ bypass: UInt8, _ freq: Float, _ q: Float, _ gain: Float, qp: UInt16 = 0) {
            d[base] = UInt8(type.rawValue); d[base + 1] = bypass; u16(base + 2, qp)
            f(base + 4, freq); f(base + 8, q); f(base + 12, gain)
        }
        let g = BULK_GLOBAL_OFFSET
        d[g + 4] = 1; d[g + 5] = 1; u16(g + 6, 0x0005); f(g + 8, 79); f(g + 12, 62)
        let c = BULK_CROSSFEED_OFFSET
        d[c] = 1; d[c + 1] = 2; d[c + 2] = 0; d[c + 3] = 0x03; f(c + 4, 650); f(c + 8, 6)
        CrossfeedPairMap.oneDACPerEar.encode(into: &d)
        for i in 0..<17 { f(BULK_DELAYS_OFFSET + i * 4, Float(i) * 0.25) }
        let xp = BULK_CROSSPOINT_OFFSET + (2 * 9 + 3) * WIRE_CROSSPOINT_SIZE
        d[xp] = 1; d[xp + 1] = 1; f(xp + 4, -4.5)
        let o = BULK_OUTPUTS_OFFSET + 2 * 12
        d[o] = 1; d[o + 1] = 1; f(o + 4, -2.5); f(o + 8, 1.25)
        d[BULK_PINS_OFFSET + 2] = 15
        band(BULK_EQ_OFFSET + (3 * 12 + 2) * WIRE_BAND_PARAMS_SIZE, .peaking, 0, 1234, 1.7, -3.5)
        band(BULK_EQ_OFFSET + (9 * 12 + 0) * WIRE_BAND_PARAMS_SIZE, .linkwitzTransform, 1, 40, 0.9, 25, qp: 384)
        band(BULK_EQ_OFFSET + (10 * 12 + 9) * WIRE_BAND_PARAMS_SIZE, .highShelf, 0, 8000, 0.7, 2)
        let name = Array("Bench 3".utf8) + [UInt8](repeating: 0, count: 25)
        d.replaceSubrange((BULK_CHANNEL_NAMES_OFFSET + 3 * 32)..<(BULK_CHANNEL_NAMES_OFFSET + 4 * 32), with: name)
        let i2s = BULK_I2S_OFFSET
        d[i2s] = 1; d[i2s + 2] = 1; d[i2s + 4] = 16; d[i2s + 5] = 21; d[i2s + 6] = 1; d[i2s + 7] = 1
        d[BULK_I2S_CLOCK_PIN_MODE_OFFSET] = 2; d[BULK_I2S_BCK_PIN_SLAVE_OFFSET] = 27
        let lv = BULK_LEVELLER_OFFSET
        d[lv] = 1; d[lv + 1] = 2; d[lv + 2] = 0; f(lv + 4, 33); f(lv + 8, 9); f(lv + 12, -60); d[lv + 16] = 0x0F; d[lv + 17] = 0x03
        f(BULK_PREAMP_OFFSET + 4, -1.5)
        f(BULK_MASTER_VOLUME_OFFSET, -12.5)
        let ic = BULK_INPUT_CONFIG_OFFSET
        d[ic] = 1; d[ic + 1] = 6; d[ic + 2] = 17; d[ic + 3] = 2; d[ic + 4] = 4; d[ic + 5] = 18; d[ic + 6] = 0; d[ic + 7] = 19
        d[ic + 8] = 23; d[ic + 9] = 0; d[ic + 10] = 24; d[ic + 11] = 1 + 0b101
        d[BULK_INPUT_I2S_CLOCK_MODE_OFFSET] = 1
        d[BULK_INPUT_ADAT_PIN_OFFSET] = 9; d[BULK_INPUT_ADAT_ENABLED_P1_OFFSET] = 2; d[BULK_INPUT_ADAT_CLOCK_MODE_P1_OFFSET] = 2
        d[BULK_LG_OFFSET] = 0
        f(BULK_USER_VOLUME_OFFSET, -6)
        let m = BULK_DAC_HW_MUTE_OFFSET
        d[m] = 1; d[m + 1] = 0; d[m + 2] = 18; u16(m + 4, 20); u16(m + 6, 30)
        band(BULK_CROSSOVER_OFFSET + (8 * 4 + 0) * WIRE_BAND_PARAMS_SIZE, .lr4_lp, 0, 2000, 0.707, 0)
        band(BULK_CROSSOVER_OFFSET + (9 * 4 + 1) * WIRE_BAND_PARAMS_SIZE, .lr4_hp, 1, 5, 0, 0)  // clamped on both sides
        d[BULK_ADAT_OFFSET] = 1; d[BULK_ADAT_OFFSET + 1] = 0
        let pb = BULK_PSYBASS_OFFSET
        d[pb] = 1; u16(pb + 2, 0x0003); f(pb + 4, 90); f(pb + 8, -3); f(pb + 12, 9); f(pb + 16, 40); f(pb + 20, -6)
        let um = BULK_UPMIX_OFFSET
        d[um] = 1; d[um + 1] = 1; d[um + 2] = 2; d[um + 3] = UInt8(bitPattern: Int8(5))
        for (k, v) in [80, 30, 40, 20, 300, 250, 10, 400, 9000, 70].enumerated() { f(um + 4 + k * 4, Float(v)) }
        let sh = BULK_SUBHARM_OFFSET
        d[sh] = 1; u16(sh + 2, 0x0101)
        for (k, v) in [-6, -3, 3, -12, 80, 200, -1].enumerated() { f(sh + 4 + k * 4, Float(v)) }
        d[sh + 32] = 1; d[sh + 33] = 0
        let tb = BULK_TUBE_OFFSET
        d[tb] = 1; d[tb + 1] = 3; d[tb + 2] = 1; d[tb + 3] = 1; u16(tb + 4, 0x0006)
        for (k, v) in [6, -20, 2, 60, 30, 5, 90, 75, -2].enumerated() { f(tb + 8 + k * 4, Float(v)) }
        for (k, (en, grp, thr, rel)) in [(UInt8(1), UInt8(1), Float(-6), Float(200)), (0, 1, -3, 50), (1, 0, -12, 500)].enumerated() {
            let r = BULK_LIMITER_OFFSET + k * WIRE_LIMITER_OUTPUT_SIZE
            d[r] = en; d[r + 1] = grp; f(r + 4, thr); f(r + 8, rel)
        }
        return d
    }

    /// SWIFT-SEAMS 8.3: `CoreMirror` writes exactly what the Swift bulk
    /// decoder writes, on the V32 capture with every section changed.
    func testSW8MirrorMatchesTheSwiftDecoderFieldByField() throws {
        let probe = FakeTransport(autoSelect: false)
        probe.selectDevice(FakeTransport.demoDevice)
        let base = try XCTUnwrap(probe.getControlRequest(request: REQ_GET_ALL_PARAMS, value: 0, index: 2,
                                                         length: BULK_PARAMS_SIZE))
        let image = Self.busyImage(base)

        func device() -> FakeTransport {
            let fake = FakeTransport(autoSelect: false)
            fake.selectDevice(FakeTransport.demoDevice)
            fake.sendControlRequest(request: REQ_SET_ALL_PARAMS, value: 0, index: 2, data: image)
            return fake
        }
        let swiftFake = device()
        let swift = connected(swiftFake) {}
        XCTAssertNil(swiftFake.coreStateSource, "the reference decodes in Swift")

        let coreFake = device()
        let core = CoreBackedTransport(link: .host(coreFake, .fake))
        let mirrored = connected(core) {}
        XCTAssertNotNil(mirrored.transport.coreStateSource, "the other reads through the core")

        let a = Self.fields(swift), b = Self.fields(mirrored)
        XCTAssertEqual(Set(a.keys), Set(b.keys))
        for key in a.keys.sorted() where key != "crossfeed" {
            XCTAssertEqual(b[key], a[key], "field \(key)")
        }
        // -- WP P1XF -- the image runs Meier with a 650 Hz / 6 dB custom pair:
        // the Swift decoder shows the custom pair, the core Meier's own values
        // (650 Hz / 9.5 dB), and the export through the core the custom pair
        let c = swift.crossfeed
        XCTAssertEqual(c.preset, 2)
        XCTAssertEqual(c.freq, 650)
        XCTAssertEqual(c.feed, 6)
        XCTAssertEqual(b["crossfeed"],
                       "\(swift.crossfeedEnabled) 2 \(c.itd) \(Float(650)) \(Float(9.5)) \(c.outputMask) \(c.pairMap.bytes)")
        let doc = PresetDocument.capture(from: mirrored)
        XCTAssertEqual(doc.crossfeed.preset, 2)
        XCTAssertEqual(doc.crossfeed.freqHz, 650)
        XCTAssertEqual(doc.crossfeed.feedDb, 6, "the device's custom feed, not Meier's 9.5")
        // The image really was the changed one on both sides.
        XCTAssertEqual(mirrored.masterVolumeDB, -12.5)
        XCTAssertEqual(mirrored.channelNames[3], "Bench 3")
        XCTAssertEqual(mirrored.channelData[9]?[0].type, .linkwitzTransform)
        XCTAssertEqual(mirrored.limiter.outputs[1].thresholdDB, -6, "ganged with output 0")
        XCTAssertFalse(mirrored.deviceReadOnly)
    }

    // MARK: - The demo device

    /// Every view's values on the core's demo device: the capture.
    func testSW8DemoDeviceShowsTheCapture() {
        let (_, vm) = demoVM()
        XCTAssertNotNil(vm.transport.coreStateSource)
        XCTAssertTrue(vm.isDeviceReady)
        XCTAssertFalse(vm.deviceReadOnly)
        XCTAssertEqual(vm.platformName, "RP2350")
        XCTAssertEqual(vm.firmwareWireFormatVersion, WIRE_FORMAT_VERSION)
        XCTAssertEqual(vm.numInputChannels, 8)
        XCTAssertEqual(vm.outputEnabled, [true, true, false, false, false, false, false, false, false])
        XCTAssertEqual(vm.masterVolumeDB, -20)
        XCTAssertEqual(Array(vm.channelNames.prefix(2)), ["USB 1", "USB 2"])
        XCTAssertEqual(vm.channelNames[8], "SPDIF 1 L")
        XCTAssertTrue(vm.matrixRouting[0][0])
        XCTAssertTrue(vm.matrixRouting[1][1])
        XCTAssertTrue(vm.lgSoundSyncEnabled)
        XCTAssertFalse(vm.crossfeedEnabled)
        XCTAssertEqual(vm.channelData.count, 17, "every wire channel has its bands")
        XCTAssertEqual(vm.channelData[0]?.count, 10)
        XCTAssertNotNil(vm.firmwareVersion)
    }

    /// A change the console did not make through its UI (another console, a
    /// knob, a write on the link) reaches the view from the core's mirror:
    /// no Swift decoder of the notification is involved.
    func testSW8DeviceChangesReachTheViewThroughTheCore() {
        let (t, vm) = demoVM()
        t.sendControlRequest(request: REQ_SET_MASTER_VOLUME, value: 0, index: 0, data: Self.float(-7))
        var packet = Data([3, 2, UInt8(FilterType.peaking.rawValue), 0])
        for v: Float in [2500, 1.4, 4.5] { packet.append(Self.float(v)) }
        t.sendControlRequest(request: REQ_SET_EQ_PARAM, value: 0, index: 0, data: packet)
        var name = Data("Desk".utf8); name.append(Data(count: 32 - name.count))
        t.sendControlRequest(request: REQ_SET_CHANNEL_NAME, value: 4, index: 2, data: name)
        XCTAssertTrue(spinUntil(5) { vm.masterVolumeDB == -7 }, "master volume: \(vm.masterVolumeDB)")
        XCTAssertTrue(spinUntil(5) { vm.channelData[3]?[2].freq == 2500 })
        XCTAssertEqual(vm.channelData[3]?[2].type, .peaking)
        XCTAssertEqual(vm.channelData[3]?[2].gain, 4.5)
        XCTAssertTrue(spinUntil(5) { vm.channelNames[4] == "Desk" })
    }

    // MARK: - Older firmware

    /// The capture as firmware with wire format V17 (1.1.5-beta3) sends it:
    /// the inverse of the core's mapping, as `wp_an2_read_only_firmware.rs`
    /// builds it.
    static func v17Image(_ v32: Data) -> Data {
        let src = [UInt8](v32)
        let size = 5872
        var d = [UInt8](repeating: 0, count: size)
        let lev = BULK_LEVELLER_OFFSET, pre = BULK_PREAMP_OFFSET, ic = BULK_INPUT_CONFIG_OFFSET, adat = BULK_ADAT_OFFSET
        let shift = 4
        d.replaceSubrange(0..<(lev + 16), with: src[0..<(lev + 16)])
        d.replaceSubrange((pre - shift)..<(adat + 8 - shift), with: src[pre..<(adat + 8)])
        let at = ic - shift
        d.replaceSubrange((at + 10)..<(at + 15), with: src[(ic + 11)..<(ic + 16)])
        d[at + 15] = 0
        d[0] = 17
        d[6] = UInt8(size & 0xFF); d[7] = UInt8(size >> 8)
        return Data(d)
    }

    /// Older firmware is shown, read-only: the core maps the V17 image, the
    /// values reach the views, the banner's flag is set, and a write from the
    /// console never reaches the device.
    func testSW8OlderFirmwareIsShownReadOnly() throws {
        let fake = FakeTransport(autoSelect: false)
        fake.selectDevice(FakeTransport.demoDevice)
        let v32 = try XCTUnwrap(fake.getControlRequest(request: REQ_GET_ALL_PARAMS, value: 0, index: 2,
                                                       length: BULK_PARAMS_SIZE))
        let old = OlderImageTransport(fake, image: Self.v17Image(v32))
        let core = CoreBackedTransport(link: .host(old, .fake))
        let vm = connected(core) {}

        XCTAssertTrue(vm.deviceReadOnly)
        XCTAssertEqual(vm.firmwareWireFormatVersion, 17)
        XCTAssertTrue(vm.isDeviceReady, "the values are shown")
        XCTAssertEqual(vm.masterVolumeDB, -20)
        XCTAssertEqual(vm.channelNames[0], "USB 1")
        XCTAssertTrue(vm.matrixRouting[0][0])
        XCTAssertFalse(vm.firmwareSupportsLevellerMasks, "V17 has no leveller masks")

        // A write is refused by the core: the device keeps its value.
        vm.masterVolumeDB = -3
        vm.sendMasterVolumeToDevice(-3)
        flush(core)
        let stored = fake.getControlRequest(request: REQ_GET_MASTER_VOLUME, value: 0, index: 0, length: 4)
        XCTAssertEqual(stored?.withUnsafeBytes { $0.load(as: Float.self) }, -20)
    }

    /// A wire format the core cannot map at all (V15) shows nothing, as
    /// before, and is still read-only.
    func testSW8UnmappedFirmwareShowsNothing() {
        let fake = FakeTransport(autoSelect: false)
        fake.selectDevice(FakeTransport.demoDevice)
        var image = Data(count: 5864)
        image[0] = 15; image[1] = 1; image[2] = 7; image[3] = 5; image[4] = 2
        image[6] = UInt8(5864 & 0xFF); image[7] = UInt8(5864 >> 8)
        let core = CoreBackedTransport(link: .host(OlderImageTransport(fake, image: image), .fake))
        let vm = DSPViewModel(transport: core)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.deviceReadOnly }, "read-only")
        settle(1)
        XCTAssertNotEqual(vm.firmwareWireFormatVersion, WIRE_FORMAT_VERSION)
        XCTAssertFalse(vm.isDeviceReady, "an image the core cannot read shows no values")
    }

    // MARK: - No echo flicker

    /// A slider drag on the master volume, -20 to -10 dB in 0.5 dB steps
    /// 30 ms apart, while each echo reaches the core 200 ms late.  Returns
    /// every value the view showed and how many core changes arrived during
    /// the drag.
    private func dragWithLateEchoes(pendingWindow: TimeInterval)
        -> (shown: [Float], duringDrag: Int, vm: DSPViewModel, fake: FakeTransport) {
        let fake = FakeTransport(autoSelect: false)
        fake.selectDevice(FakeTransport.demoDevice)
        let core = CoreBackedTransport(link: .host(LateEchoTransport(fake, delay: 0.2), .fake))
        let vm = connected(core) {}
        XCTAssertEqual(vm.masterVolumeDB, -20)
        vm.coreMirror.pendingWindow = pendingWindow

        var shown: [Float] = []
        let watch = vm.$masterVolumeDB.sink { shown.append($0) }
        var coreChanges = 0
        let changes = core.addStateObserver { _ in coreChanges += 1 }
        defer { watch.cancel(); changes.cancel() }

        // What a slider drag does: the value on screen, then the write.
        for step in 1...20 {
            let v = -20 + Float(step) * 0.5
            vm.masterVolumeDB = v
            vm.sendMasterVolumeToDevice(v)
            RunLoop.current.run(until: Date().addingTimeInterval(0.03))
        }
        let duringDrag = coreChanges
        XCTAssertTrue(spinUntil(5) { core.readState().masterVolumeDb == -10 }, "the last echo reached the core's mirror")
        settle(CoreMirror.defaultPendingWindow + 0.3)
        return (shown, duringDrag, vm, fake)
    }

    /// No echo flicker: the core's mirror moves through the stale positions,
    /// the view never steps back, and it ends on the last position, as the
    /// device does.
    func testSW8NoEchoFlickerWhileDragging() {
        let (shown, duringDrag, vm, fake) = dragWithLateEchoes(pendingWindow: CoreMirror.defaultPendingWindow)
        XCTAssertGreaterThan(duringDrag, 0, "stale echoes arrived while dragging")
        XCTAssertEqual(shown, shown.sorted(), "the view stepped back: \(shown)")
        XCTAssertEqual(vm.masterVolumeDB, -10)
        let stored = fake.getControlRequest(request: REQ_GET_MASTER_VOLUME, value: 0, index: 0, length: 4)
        XCTAssertEqual(stored?.withUnsafeBytes { $0.load(as: Float.self) }, -10)
    }

    /// The control: the same drag without the pending-key rule does step
    /// back (so the test above can see flicker), and still ends right.
    func testSW8WithoutThePendingRuleTheSameDragFlickers() {
        let (shown, _, vm, _) = dragWithLateEchoes(pendingWindow: 0)
        XCTAssertNotEqual(shown, shown.sorted(), "late echoes pulled the view back: \(shown)")
        XCTAssertEqual(vm.masterVolumeDB, -10)
    }

    /// The pending-key rule on its own: a field edited locally ignores a
    /// differing core value for 500 ms, takes it after that, and stops being
    /// pending once the core reports the edited value.
    func testSW8PendingKeyRule() {
        let source = CoreBackedTransport(link: .demo)
        source.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(10) { source.isConnected })
        _ = source.readAll()
        let base = source.readState()

        let vm = DSPViewModel(transport: FakeTransport(autoSelect: false))   // never connected
        let mirror = vm.coreMirror
        var clock: TimeInterval = 1000
        mirror.now = { clock }
        mirror.apply(base, force: true)
        XCTAssertEqual(vm.masterVolumeDB, base.masterVolumeDb)

        func state(_ db: Float) -> DspState { var s = base; s.masterVolumeDb = db; return s }

        vm.masterVolumeDB = -5                                   // a local edit
        clock += 0.1
        mirror.apply(state(-12), force: false)                   // a stale echo
        XCTAssertEqual(vm.masterVolumeDB, -5, "held back while pending")
        XCTAssertEqual(mirror.deferredKeys, ["master_volume_db"])
        clock += 0.5
        mirror.apply(state(-12), force: false)                   // the window is over
        XCTAssertEqual(vm.masterVolumeDB, -12)
        XCTAssertEqual(mirror.deferredKeys, [])

        vm.masterVolumeDB = -3
        clock += 0.1
        mirror.apply(state(-3), force: false)                    // the echo of the edit
        mirror.apply(state(-4), force: false)                    // then another console
        XCTAssertEqual(vm.masterVolumeDB, -4, "no longer pending once the core agreed")

        vm.masterVolumeDB = -1
        mirror.apply(state(-8), force: true)                     // a whole re-read wins
        XCTAssertEqual(vm.masterVolumeDB, -8)

        vm.masterVolumeDB = -2
        mirror.reset()                                           // a new connection
        mirror.apply(state(-9), force: false)
        XCTAssertEqual(vm.masterVolumeDB, -9)

        // One channel's EQ edit holds back only that channel.
        var edited = vm.channelData[0] ?? []
        edited[0].gain = 6
        vm.channelData[0] = edited
        var s = state(-9)
        s.filters[0][0].gain = 1
        s.filters[1][0].gain = 2
        mirror.apply(s, force: false)
        XCTAssertEqual(vm.channelData[0]?[0].gain, 6)
        XCTAssertEqual(vm.channelData[1]?[0].gain, 2)
    }

    // MARK: - Restore-verified writes

    /// The MIDI and Link hardware sequence (RestoreVerifiedWrites) on the
    /// core's demo device: every write reads back, every restore is verified,
    /// and the device, the core's mirror and the view end on the reference.
    func testSW8RestoreVerifiedWritesOnTheDemoDevice() throws {
        let (t, vm) = demoVM()
        let name7 = vm.channelNames[7]
        let writes = RestoreVerifiedWrites(transport: t)
        let observer = writes.observe()
        defer { observer.cancel() }
        let reference = try XCTUnwrap(writes.captureReference(numOutputs: 9), "the demo device's reference")
        let failure = writes.run(bulkRef: reference.bulk, configRef: reference.config, timeout: 120)
        XCTAssertNil(failure, writes.report.joined(separator: "\n"))
        XCTAssertTrue(writes.report.contains { $0.hasPrefix("end: the core's mirror is byte-identical") },
                      writes.report.joined(separator: "\n"))
        settle(CoreMirror.defaultPendingWindow)
        XCTAssertEqual(vm.masterVolumeDB, -20, "the view is back on the reference")
        XCTAssertEqual(vm.channelNames[7], name7)
    }

    // MARK: - Presets and refusals

    /// A preset loaded on the device (another console, an IR button): the
    /// core re-reads it, the view shows it, and the unsaved-changes baseline
    /// follows, so the loaded preset is not dirty.
    func testSW8DevicePresetLoadRebasesTheBaseline() {
        let (t, vm) = demoVM()
        // P1TZF: the save through its typed call and a token (the raw request refuses it).
        XCTAssertNoThrow(try t.call { try $0.savePresetConfirmed(slot: 3, token: $0.confirmToken(op: .savePreset)) }.get())
        var packet = Data([0, 0, UInt8(FilterType.peaking.rawValue), 0])
        for v: Float in [440, 2, -6] { packet.append(Self.float(v)) }
        t.sendControlRequest(request: REQ_SET_EQ_PARAM, value: 0, index: 0, data: packet)
        XCTAssertTrue(spinUntil(5) { vm.channelData[0]?[0].freq == 440 })
        XCTAssertTrue(vm.hasUnsavedChanges, "an EQ change against the baseline")

        XCTAssertEqual(t.getControlRequest(request: REQ_PRESET_LOAD, value: 3, index: 2, length: 1), Data([PRESET_OK]))
        XCTAssertTrue(spinUntil(10) { vm.channelData[0]?[0].freq != 440 && vm.activePresetSlot == 3 },
                      "the loaded preset is shown")
        XCTAssertTrue(spinUntil(5) { !vm.hasUnsavedChanges }, "and is the new baseline")
    }

    /// A write the device refused (RefusedWrite's resync, a BULK_INVALIDATED):
    /// the core's value goes back on screen over the edit.
    func testSW8RefusedWriteRevertsToTheCore() {
        let (_, vm) = demoVM()
        vm.masterVolumeDB = -1                                   // shown, never written
        AppState.shared.interruptMonitor.onBulkInvalidated?(0)   // this view model's hook
        XCTAssertTrue(spinUntil(5) { vm.masterVolumeDB == -20 })
    }
}

/// Answers the bulk read with another firmware's image; everything else goes
/// to the fake.
final class OlderImageTransport: ForwardingTransport {
    private let image: Data
    init(_ inner: any DeviceTransport, image: Data) {
        self.image = image
        super.init(inner)
    }
    override func getControlResult(request: UInt8, value: UInt16, index: UInt16, length: UInt16) -> Result<Data, DSPi_Console.LinkStatus> {
        guard request == REQ_GET_ALL_PARAMS, inner.isConnected else {
            return super.getControlResult(request: request, value: value, index: index, length: length)
        }
        return .success(image.prefix(Int(length)))
    }
}

/// Delivers the device's notifications `delay` late, as a busy link does.
final class LateEchoTransport: ForwardingTransport {
    private let delay: TimeInterval
    init(_ inner: any DeviceTransport, delay: TimeInterval) {
        self.delay = delay
        super.init(inner)
    }
    override func addNotificationObserver(_ handler: @escaping (LinkNotification) -> Void) -> AnyCancellable {
        let delay = self.delay
        return super.addNotificationObserver { note in
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) { handler(note) }
        }
    }
}
