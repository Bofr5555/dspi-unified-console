import XCTest
import dspi_core
import dspi_constants
@testable import DSPi_Console

/// WP SW10: the effects (loudness, crossfeed and its pair map, psychoacoustic
/// bass, subharmonic synth, tube, limiter, upmixer, leveller) through the
/// core's typed setters (Commands.swift, ExtensionCommands.swift), their
/// drag-time sends through SW9's write lane.  The edits go through the view
/// model as the views make them, and Undo and Redo step them back through
/// the core's journal.
final class CoreDomainSW10Tests: XCTestCase, CoreDomainTesting {

    /// `a`, or `b` when the setting already holds `a`: an edit that changes it.
    private func other<T: Equatable>(_ current: T, _ a: T, _ b: T) -> T { current == a ? b : a }

    // MARK: - Undo and Redo, one effect at a time

    func testSW10UndoAndRedoALoudnessEdit() {
        let (_, _, vm) = demoVM()
        XCTAssertFalse(vm.undo.canUndo, "a new connection has nothing to undo")
        let ref = vm.loudness.refSPL
        let to = other(ref, Float(70), 90)
        vm.setLoudnessRef(to)
        undoAndRedo(vm, undone: { vm.loudness.refSPL == ref }, redone: { vm.loudness.refSPL == to })

        let on = vm.loudnessEnabled
        vm.setLoudness(!on)
        undoAndRedo(vm, undone: { vm.loudnessEnabled == on }, redone: { vm.loudnessEnabled == !on })
    }

    func testSW10UndoAndRedoACrossfeedEdit() {
        let (_, _, vm) = demoVM()
        let feed = vm.crossfeed.feed
        let to = other(feed, Float(8), 12)
        vm.setCrossfeedFeed(to)
        undoAndRedo(vm, undone: { vm.crossfeed.feed == feed }, redone: { vm.crossfeed.feed == to })

        let itd = vm.crossfeed.itd
        vm.setCrossfeedITD(!itd)
        undoAndRedo(vm, undone: { vm.crossfeed.itd == itd }, redone: { vm.crossfeed.itd == !itd })
    }

    func testSW10UndoAndRedoAPsychoacousticBassEdit() {
        let (_, _, vm) = demoVM()
        let cutoff = vm.psybass.cutoffHz
        let to = other(cutoff, Float(120), 150)
        vm.setPsybassCutoff(to)
        undoAndRedo(vm, undone: { vm.psybass.cutoffHz == cutoff }, redone: { vm.psybass.cutoffHz == to })

        let mask = vm.psybass.outputMask
        vm.setPsybassOutputChannel(1, enabled: mask & 0b10 == 0)
        let edited = mask ^ 0b10
        undoAndRedo(vm, undone: { vm.psybass.outputMask == mask }, redone: { vm.psybass.outputMask == edited })
    }

    func testSW10UndoAndRedoASubharmEdit() {
        let (_, _, vm) = demoVM()
        let low = vm.subharm.lowDB
        let to = other(low, Float(-6), 3)
        vm.setSubharmLow(to)
        undoAndRedo(vm, undone: { vm.subharm.lowDB == low }, redone: { vm.subharm.lowDB == to })

        let mode = vm.subharm.selectMode
        let toMode = other(mode, SUBHARM_SELECT_PERCUSSIVE, SUBHARM_SELECT_SUSTAINED)
        vm.setSubharmSelectMode(toMode)
        undoAndRedo(vm, undone: { vm.subharm.selectMode == mode }, redone: { vm.subharm.selectMode == toMode })
    }

    func testSW10UndoAndRedoATubeEdit() {
        let (_, _, vm) = demoVM()
        let drive = vm.tube.driveDB
        let to = other(drive, Float(6), 12)
        vm.setTubeDrive(to)
        undoAndRedo(vm, undone: { vm.tube.driveDB == drive }, redone: { vm.tube.driveDB == to })
    }

    /// A tube type loads its row into the four character knobs.  Undo puts
    /// the type and the knobs back: the core journals the row as the type's
    /// companions.
    func testSW10UndoOfATubeTypeRestoresItsCharacter() throws {
        let (_, _, vm) = demoVM()
        let (type, bias, sag) = (vm.tube.type, vm.tube.biasPct, vm.tube.sagPct)
        let toType = (1...TUBE_TYPE_MAX).first { t in
            guard t != type, t < TUBE_TYPE_ROWS.count, let row = TUBE_TYPE_ROWS[t] else { return false }
            return row.biasPct != bias || row.sagPct != sag
        }
        let t = try XCTUnwrap(toType, "a type whose row differs from the knobs")
        let row = try XCTUnwrap(TUBE_TYPE_ROWS[t])
        vm.setTubeType(t)
        XCTAssertEqual(vm.tube.biasPct, row.biasPct, "the row is shown at once")
        undoAndRedo(vm,
                    undone: { vm.tube.type == type && vm.tube.biasPct == bias && vm.tube.sagPct == sag },
                    redone: { vm.tube.type == t && vm.tube.biasPct == row.biasPct && vm.tube.sagPct == row.sagPct })
    }

    /// The limiter's threshold.  The enable is an audible change: the core
    /// records it and Undo steps over it, as in Qt.
    func testSW10UndoAndRedoALimiterEdit() {
        let (core, t, vm) = demoVM()
        let threshold = vm.limiter.outputs[1].thresholdDB
        let to = other(threshold, Float(-12), -6)
        vm.sendLimiterParamToDevice(output: 1, LIMITER_PARAM_THRESHOLD_DB, to)   // a drag, then its commit
        vm.setLimiterThreshold(output: 1, to)
        XCTAssertTrue(entries(core, t, "raw").isEmpty, "no limiter write went out raw")
        undoAndRedo(vm, undone: { vm.limiter.outputs[1].thresholdDB == threshold },
                    redone: { vm.limiter.outputs[1].thresholdDB == to })
    }

    /// A linked group is moved by the core (Swift's LimiterGang is not what
    /// the view shows through a core): a threshold on one member reaches the
    /// other, and one Undo puts both back.
    func testSW10ALimiterGroupMovesTogetherAndUndoesTogether() {
        let (core, t, vm) = demoVM()
        vm.setLimiterLinkGroup(output: 0, 2)
        vm.setLimiterLinkGroup(output: 1, 2)
        XCTAssertTrue(spinUntil(5) { vm.limiter.outputs[1].linkGroup == 2 })
        let before = vm.limiter.outputs[0].thresholdDB
        XCTAssertTrue(spinUntil(5) { vm.limiter.outputs[1].thresholdDB == before }, "the joiner adopts the group's")
        let to = other(before, Float(-15), -9)
        vm.setLimiterThreshold(output: 0, to)
        XCTAssertTrue(spinUntil(5) { vm.limiter.outputs[0].thresholdDB == to && vm.limiter.outputs[1].thresholdDB == to })
        XCTAssertEqual(entries(core, t, "limit.threshold").count, 1, "journalled")
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo })
        vm.undo.undo()
        XCTAssertTrue(spinUntil(5) {
            vm.limiter.outputs[0].thresholdDB == before && vm.limiter.outputs[1].thresholdDB == before
        }, "both members back: \(vm.limiter.outputs[0].thresholdDB), \(vm.limiter.outputs[1].thresholdDB)")
    }

    func testSW10UndoAndRedoAnUpmixerEdit() {
        let (_, _, vm) = demoVM()
        let strength = vm.upmix.strengthPct
        let to = other(strength, Float(40), 60)
        vm.setUpmixStrength(to)
        undoAndRedo(vm, undone: { vm.upmix.strengthPct == strength }, redone: { vm.upmix.strengthPct == to })

        let mode = vm.upmix.surroundMode
        let toMode = other(mode, 1, 2)
        vm.setUpmixSurroundMode(toMode)
        undoAndRedo(vm, undone: { vm.upmix.surroundMode == mode }, redone: { vm.upmix.surroundMode == toMode })
    }

    func testSW10UndoAndRedoALevellerEdit() {
        let (_, _, vm) = demoVM()
        let amount = vm.leveller.amount
        let to = other(amount, Float(30), 70)
        vm.setLevellerAmount(to)
        undoAndRedo(vm, undone: { vm.leveller.amount == amount }, redone: { vm.leveller.amount == to })

        let speed = vm.leveller.speed
        let toSpeed = other(speed, 0, 2)
        vm.setLevellerSpeed(toSpeed)
        undoAndRedo(vm, undone: { vm.leveller.speed == speed }, redone: { vm.leveller.speed == toSpeed })
    }

    /// An effect slider's drag (ParameterRow's live closure, then its commit
    /// on release): many lane writes, one Undo step.
    func testSW10AnEffectSliderDragIsOneUndoStep() {
        let (core, t, vm) = demoVM()
        let before = vm.psybass.harmonicsDB
        t.setDragging(true)
        for step in 1...10 {
            vm.sendFloatParamToDevice(REQ_SET_PSYBASS_HARMONICS, -12 + Float(step))
            RunLoop.current.run(until: Date().addingTimeInterval(0.01))
        }
        vm.setPsybassHarmonics(-2)
        t.setDragging(false)
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo })
        XCTAssertTrue(entries(core, t, "raw").isEmpty, "no drag write went out raw")
        XCTAssertFalse(entries(core, t, "bass.harmonics").isEmpty, "the drag went to the typed setter")
        vm.undo.undo()
        XCTAssertTrue(spinUntil(5) { vm.psybass.harmonicsDB == before }, "one step: \(vm.psybass.harmonicsDB)")
        XCTAssertTrue(spinUntil(5) { !vm.undo.canUndo })
    }

    // MARK: - No raw writes in this domain

    private static func float(_ fake: FakeTransport, _ request: UInt8, _ value: UInt16 = 0) -> Float? {
        guard let d = fake.getControlRequest(request: request, value: value, index: HostBridge.vendorInterface,
                                             length: 4), d.count >= 4 else {
            return nil
        }
        return d.withUnsafeBytes { $0.load(as: Float.self) }
    }

    private static func byte(_ fake: FakeTransport, _ request: UInt8) -> UInt8? {
        fake.getControlRequest(request: request, value: 0, index: HostBridge.vendorInterface, length: 1)?.first
    }

    /// Swift's FakeTransport under the core, with the effects the core
    /// probes for.  The fake answers a GET only once its SET has run (its
    /// echo store), and the core probes each effect with a GET when it
    /// connects: give those probes their answers, then connect again.  (The
    /// limiter's probe has no SET twin in the fake: its writes are checked
    /// on the core's demo device.)
    private func effectsHostVM() -> (FakeTransport, DspiCore, CoreBackedTransport, DSPViewModel) {
        let (fake, core, t, vm) = hostVM()
        let probes: [(UInt8, UInt16, Data)] = [
            (REQ_SET_LOUDNESS_MASK, 0, Data([0xFF, 0xFF])), (REQ_SET_CROSSFEED_OUTPUTS, 0, Data([0x0F])),
            (REQ_SET_LEVELLER_MASKS, 0, Data([0xFF, 0xFF])), (REQ_SET_PSYBASS, 0, Data([0])),
            (REQ_SET_SUBHARM, 0, Data([0])), (REQ_SET_TUBE_PARAM, TUBE_PARAM_ENABLED, Data(count: 4)),
        ]
        for (request, value, data) in probes {
            fake.sendControlRequest(request: request, value: value, index: HostBridge.vendorInterface, data: data)
        }
        fake.disconnect()   // the generation moves at once
        fake.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == t.generation },
                      "connected and read again")
        settle()
        return (fake, core, t, vm)
    }

    /// Every setter and live send of the effects goes to a typed core call:
    /// the journal holds no raw entry, each lands under its own path, and
    /// the device (Swift's FakeTransport, through the core) holds the values.
    func testSW10EveryEffectWriteIsTypedAndReachesTheDevice() {
        let (fake, core, t, vm) = effectsHostVM()

        vm.setLoudness(true)
        vm.setLoudnessRef(75)
        vm.sendFloatParamToDevice(REQ_SET_LOUDNESS_INTENSITY, 120)
        vm.setLoudnessIntensity(130)
        vm.setLoudnessMask(0b11)
        vm.setCrossfeed(true)
        vm.setCrossfeedPreset(3)
        vm.sendFloatParamToDevice(REQ_SET_CROSSFEED_FREQ, 800)
        vm.setCrossfeedFreq(900)
        vm.setCrossfeedFeed(7)
        let itd = !vm.crossfeed.itd   // a change, so it is journalled
        vm.setCrossfeedITD(itd)
        let pairs = vm.crossfeed.outputMask ^ 0b1
        vm.setCrossfeedMask(pairs)
        vm.setPsybass(true)
        vm.setPsybassCutoff(110)
        vm.setPsybassDrive(9)
        vm.setPsybassMask(0b101)
        vm.setSubharm(true)
        vm.setSubharmHigh(-4)
        vm.sendFloatParamToDevice(REQ_SET_SUBHARM_CEILING, -10)
        vm.setSubharmCeiling(-12)
        let linked = !vm.subharm.linkPairs
        vm.setSubharmLinkPairs(linked)
        vm.setSubharmSolo(true)
        vm.setSubharmMask(0b110)
        vm.setTube(true)
        vm.sendTubeParamToDevice(TUBE_PARAM_MIX_PCT, 40, TUBE_MIX_MIN, TUBE_MIX_MAX)
        vm.setTubeMix(50)
        vm.setUpmixEnabled(true)
        vm.sendUpmixParamToDevice(UPMIX_PARAM_ATTACK, 30)
        vm.setUpmixAttack(35)
        vm.setLeveller(true)
        vm.setLevellerGate(-50)
        let lookahead = !vm.leveller.lookahead
        vm.setLevellerLookahead(lookahead)
        vm.setLevellerMasks(detector: 0b11, apply: 0b01)
        flush(t)
        settle()

        XCTAssertTrue(entries(core, t, "raw").isEmpty, "no effect write went out raw")
        for path in ["loud.on", "loud.ref", "loud.intensity", "loud.mask", "cf.on", "cf.preset", "cf.freq", "cf.feed",
                     "cf.itd", "cf.outputs", "bass.on", "bass.cutoff", "bass.drive", "bass.mask", "sub.on", "sub.high",
                     "sub.ceiling", "sub.link", "sub.solo", "sub.mask", "tube.on", "tube.mix", "up.on", "up.attack", "lev.on", "lev.gate", "lev.lookahead", "lev.masks"] {
            XCTAssertFalse(entries(core, t, path).isEmpty, "\(path) went through its typed setter")
        }

        // What the device holds, read straight from the fake.
        XCTAssertEqual(Self.byte(fake, REQ_GET_LOUDNESS), 1)
        XCTAssertEqual(Self.float(fake, REQ_GET_LOUDNESS_REF), 75)
        XCTAssertEqual(Self.float(fake, REQ_GET_LOUDNESS_INTENSITY), 130)
        XCTAssertEqual(Self.byte(fake, REQ_GET_CROSSFEED_PRESET), 3)
        XCTAssertEqual(Self.float(fake, REQ_GET_CROSSFEED_FREQ), 900)
        XCTAssertEqual(Self.float(fake, REQ_GET_CROSSFEED_FEED), 7)
        XCTAssertEqual(Self.byte(fake, REQ_GET_CROSSFEED_ITD), itd ? 1 : 0)
        XCTAssertEqual(Self.byte(fake, REQ_GET_PSYBASS), 1)
        XCTAssertEqual(Self.float(fake, REQ_GET_PSYBASS_CUTOFF), 110)
        XCTAssertEqual(Self.float(fake, REQ_GET_PSYBASS_DRIVE), 9)
        XCTAssertEqual(Self.byte(fake, REQ_GET_SUBHARM), 1)
        XCTAssertEqual(Self.float(fake, REQ_GET_SUBHARM_HIGH), -4)
        XCTAssertEqual(Self.float(fake, REQ_GET_SUBHARM_CEILING), -12)
        XCTAssertEqual(Self.byte(fake, REQ_GET_SUBHARM_LINK), linked ? 1 : 0)
        XCTAssertEqual(Self.byte(fake, REQ_GET_SUBHARM_SOLO), 1)
        XCTAssertEqual(Self.float(fake, REQ_GET_TUBE_PARAM, TUBE_PARAM_MIX_PCT), 50)
        XCTAssertEqual(Self.byte(fake, REQ_GET_LEVELLER), 1)
        XCTAssertEqual(Self.float(fake, REQ_GET_LEVELLER_GATE), -50)
        XCTAssertEqual(Self.byte(fake, REQ_GET_LEVELLER_LOOKAHEAD), lookahead ? 1 : 0)

        // And the core's mirror, which the view model shows.
        let s = t.readState()
        XCTAssertEqual(s.upmix[Int(DspiConstants.UPMIX_PARAM_ATTACK)], 35)
        XCTAssertEqual(s.upmix[Int(DspiConstants.UPMIX_PARAM_ENABLED)], 1)
        XCTAssertEqual(s.subharm[Int(DspiConstants.SUBHARM_PARAM_MASK)], 6)
        XCTAssertEqual(s.psybassOutputMask, 0b101)
        XCTAssertEqual(s.loudnessOutputMask, 0b11)
        XCTAssertEqual(s.levellerDetectorMask, 0b11)
        XCTAssertEqual(s.levellerApplyMask, 0b01)
    }

    /// Every float SET a view sends live has its typed setter (none of them
    /// falls back to raw through a core).
    func testSW10EveryLiveFloatRequestHasATypedSetter() {
        let live: [UInt8] = [REQ_SET_LOUDNESS_REF, REQ_SET_LOUDNESS_INTENSITY, REQ_SET_CROSSFEED_FREQ,
                             REQ_SET_CROSSFEED_FEED, REQ_SET_PSYBASS_CUTOFF, REQ_SET_PSYBASS_HARMONICS,
                             REQ_SET_PSYBASS_DRIVE, REQ_SET_PSYBASS_CHARACTER, REQ_SET_PSYBASS_ORIGINAL,
                             REQ_SET_SUBHARM_LOW, REQ_SET_SUBHARM_HIGH, REQ_SET_SUBHARM_TOP, REQ_SET_SUBHARM_BOOST,
                             REQ_SET_SUBHARM_DEPTH, REQ_SET_SUBHARM_HOLD, REQ_SET_SUBHARM_CEILING,
                             REQ_SET_LEVELLER_AMOUNT, REQ_SET_LEVELLER_MAXGAIN, REQ_SET_LEVELLER_GATE]
        for request in live {
            XCTAssertNotNil(DSPViewModel.typedFloatSet(request), String(format: "0x%02X", request))
        }
        XCTAssertEqual(Set(live.compactMap { DSPViewModel.typedFloatSet($0)?.lane }).count, live.count,
                       "one lane key each")
    }

    /// The crossfeed pair map: written and read back by the core (it checks
    /// the map, sends it normalized and reads it back), and an invalid one
    /// refused before anything is sent.
    func testSW10ThePairMapGoesThroughTheCore() {
        let (_, core, t, vm) = hostVM()   // the fake answers like the bofr fork
        DispatchQueue.global().sync { vm.probeExtensions(force: true) }
        XCTAssertTrue(spinUntil(5) { vm.crossfeedPairMapSupported }, "the fake lists the pair map")
        let before = vm.crossfeed.pairMap
        XCTAssertNotEqual(before, .oneDACPerEar)
        XCTAssertTrue(vm.setCrossfeedPairMap(.oneDACPerEar))
        XCTAssertTrue(spinUntil(5) { vm.crossfeed.pairMap == .oneDACPerEar }, "the core's read-back is shown")
        XCTAssertTrue(entries(core, t, "raw").isEmpty, "the map did not go raw")
        XCTAssertEqual(DispatchQueue.global().sync { vm.readCrossfeedPairMap() }, .oneDACPerEar)
        XCTAssertFalse(vm.setCrossfeedPairMap(CrossfeedPairMap(bytes: [0x11, 0, 0, 0])), "invalid: not sent")

        // Live routing, as a crossfeed setting: Undo puts the pairing back.
        undoAndRedo(vm, undone: { vm.crossfeed.pairMap == before }, redone: { vm.crossfeed.pairMap == .oneDACPerEar })
    }

    // MARK: - Reads and polls through the core

    /// The subharm meter and solo on the core's demo device, through the
    /// core's calls (the poll timer's path).
    func testSW10SubharmMeterAndSoloThroughTheCore() {
        let (_, t, vm) = demoVM()
        vm.subharm.meter.levels = []
        DispatchQueue.global().async { vm.fetchSubharmMeter() }
        XCTAssertTrue(spinUntil(5) { vm.subharm.meter.levels.count == vm.numOutputChannels },
                      "one level per output: \(vm.subharm.meter.levels.count)")

        vm.setSubharmSolo(true)
        flush(t)
        vm.subharm.solo = false
        DispatchQueue.global().async { vm.fetchSubharmSolo() }
        XCTAssertTrue(spinUntil(5) { vm.subharm.solo }, "the core's copy")
        vm.setSubharmSolo(false)
    }
}
